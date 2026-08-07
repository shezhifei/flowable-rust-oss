use crate::error::FlowableError;
use crate::scripting::ast::*;
use crate::scripting::tokenizer::Token;
use serde_json::Value;

/// Recursive descent parser that converts a token stream into an AST.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    /// Parse the entire token stream into a list of statements.
    pub fn parse(&mut self) -> Result<Vec<Statement>, FlowableError> {
        let mut statements = Vec::new();
        while !self.is_at_end() {
            statements.push(self.parse_statement()?);
        }
        Ok(statements)
    }

    // ── Helpers ──────────────────────────────────────────────

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.pos);
        self.pos += 1;
        token
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn expect(&mut self, expected: &Token) -> Result<(), FlowableError> {
        match self.peek() {
            Some(t) if t == expected => {
                self.advance();
                Ok(())
            }
            Some(t) => Err(FlowableError::ExecutionError(format!(
                "Expected {:?}, found {:?}",
                expected, t
            ))),
            None => Err(FlowableError::ExecutionError(format!(
                "Expected {:?}, found end of script",
                expected
            ))),
        }
    }

    fn expect_identifier(&mut self) -> Result<String, FlowableError> {
        match self.advance().cloned() {
            Some(Token::Identifier(name)) => Ok(name),
            Some(other) => Err(FlowableError::ExecutionError(format!(
                "Expected identifier, found {:?}",
                other
            ))),
            None => Err(FlowableError::ExecutionError(
                "Expected identifier, found end of script".to_string(),
            )),
        }
    }

    fn skip_semicolons(&mut self) {
        while self.peek() == Some(&Token::Semicolon) {
            self.advance();
        }
    }

    // ── Statements ──────────────────────────────────────────

    fn parse_statement(&mut self) -> Result<Statement, FlowableError> {
        self.skip_semicolons();
        if self.is_at_end() {
            return Ok(Statement::Block(Vec::new()));
        }

        match self.peek() {
            Some(Token::Var | Token::Let) => self.parse_var_decl(),
            Some(Token::If) => self.parse_if_stmt(),
            Some(Token::For) => self.parse_for_stmt(),
            Some(Token::While) => self.parse_while_stmt(),
            Some(Token::Function) => self.parse_function_decl(),
            Some(Token::Return) => self.parse_return_stmt(),
            Some(Token::LeftBrace) => self.parse_block_stmt(),
            _ => self.parse_expression_stmt(),
        }
    }

    fn parse_var_decl(&mut self) -> Result<Statement, FlowableError> {
        self.advance(); // skip var/let
        let name = self.expect_identifier()?;
        let initializer = if self.peek() == Some(&Token::Assign) {
            self.advance(); // skip =
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.skip_semicolons();
        Ok(Statement::VarDecl { name, initializer })
    }

    fn parse_if_stmt(&mut self) -> Result<Statement, FlowableError> {
        self.advance(); // skip if
        self.expect(&Token::LeftParen)?;
        let condition = self.parse_expression()?;
        self.expect(&Token::RightParen)?;
        let then_body = self.parse_body()?;
        let else_body = if self.peek() == Some(&Token::Else) {
            self.advance(); // skip else
            if self.peek() == Some(&Token::If) {
                // else if
                let nested_if = self.parse_if_stmt()?;
                Some(vec![nested_if])
            } else {
                Some(self.parse_body()?)
            }
        } else {
            None
        };
        Ok(Statement::IfStmt {
            condition,
            then_body,
            else_body,
        })
    }

    fn parse_for_stmt(&mut self) -> Result<Statement, FlowableError> {
        self.advance(); // skip for
        self.expect(&Token::LeftParen)?;

        // init
        let init = if self.peek() == Some(&Token::Semicolon) {
            self.advance();
            None
        } else {
            let stmt = self.parse_statement()?;
            self.skip_semicolons();
            Some(Box::new(stmt))
        };

        // condition
        let condition = if self.peek() == Some(&Token::Semicolon) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.skip_semicolons();

        // update
        let update = if self.peek() == Some(&Token::RightParen) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect(&Token::RightParen)?;

        let body = self.parse_body()?;
        Ok(Statement::ForStmt {
            init,
            condition,
            update,
            body,
        })
    }

    fn parse_while_stmt(&mut self) -> Result<Statement, FlowableError> {
        self.advance(); // skip while
        self.expect(&Token::LeftParen)?;
        let condition = self.parse_expression()?;
        self.expect(&Token::RightParen)?;
        let body = self.parse_body()?;
        Ok(Statement::WhileStmt { condition, body })
    }

    fn parse_function_decl(&mut self) -> Result<Statement, FlowableError> {
        self.advance(); // skip function
        let name = self.expect_identifier()?;
        self.expect(&Token::LeftParen)?;
        let mut params = Vec::new();
        if self.peek() != Some(&Token::RightParen) {
            params.push(self.expect_identifier()?);
            while self.peek() == Some(&Token::Comma) {
                self.advance();
                params.push(self.expect_identifier()?);
            }
        }
        self.expect(&Token::RightParen)?;
        let body = self.parse_body()?;
        Ok(Statement::FunctionDecl { name, params, body })
    }

    fn parse_return_stmt(&mut self) -> Result<Statement, FlowableError> {
        self.advance(); // skip return
        let value = if self.peek() == Some(&Token::Semicolon) || self.is_at_end() {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.skip_semicolons();
        Ok(Statement::ReturnStmt(value))
    }

    fn parse_block_stmt(&mut self) -> Result<Statement, FlowableError> {
        let body = self.parse_body()?;
        Ok(Statement::Block(body))
    }

    fn parse_body(&mut self) -> Result<Vec<Statement>, FlowableError> {
        if self.peek() == Some(&Token::LeftBrace) {
            self.advance(); // skip {
            let mut stmts = Vec::new();
            while self.peek() != Some(&Token::RightBrace) && !self.is_at_end() {
                stmts.push(self.parse_statement()?);
            }
            self.expect(&Token::RightBrace)?;
            Ok(stmts)
        } else {
            // Single statement body
            Ok(vec![self.parse_statement()?])
        }
    }

    fn parse_expression_stmt(&mut self) -> Result<Statement, FlowableError> {
        let expr = self.parse_expression()?;
        self.skip_semicolons();
        Ok(Statement::ExpressionStmt(expr))
    }

    // ── Expressions (precedence climbing) ───────────────────

    fn parse_expression(&mut self) -> Result<Expression, FlowableError> {
        self.parse_assignment()
    }

    fn parse_assignment(&mut self) -> Result<Expression, FlowableError> {
        let expr = self.parse_or()?;

        match self.peek() {
            Some(Token::Assign) => {
                self.advance();
                let value = self.parse_assignment()?;
                match expr {
                    Expression::Variable(name) => Ok(Expression::Assignment {
                        name,
                        value: Box::new(value),
                    }),
                    _ => Err(FlowableError::ExecutionError(
                        "Invalid assignment target".to_string(),
                    )),
                }
            }
            Some(Token::PlusAssign) => {
                self.advance();
                let value = self.parse_assignment()?;
                match expr {
                    Expression::Variable(name) => Ok(Expression::CompoundAssignment {
                        name,
                        operator: BinaryOperator::Add,
                        value: Box::new(value),
                    }),
                    _ => Err(FlowableError::ExecutionError(
                        "Invalid compound assignment target".to_string(),
                    )),
                }
            }
            Some(Token::MinusAssign) => {
                self.advance();
                let value = self.parse_assignment()?;
                match expr {
                    Expression::Variable(name) => Ok(Expression::CompoundAssignment {
                        name,
                        operator: BinaryOperator::Sub,
                        value: Box::new(value),
                    }),
                    _ => Err(FlowableError::ExecutionError(
                        "Invalid compound assignment target".to_string(),
                    )),
                }
            }
            _ => Ok(expr),
        }
    }

    fn parse_or(&mut self) -> Result<Expression, FlowableError> {
        let mut left = self.parse_and()?;
        while self.peek() == Some(&Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                operator: BinaryOperator::Or,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expression, FlowableError> {
        let mut left = self.parse_equality()?;
        while self.peek() == Some(&Token::And) {
            self.advance();
            let right = self.parse_equality()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                operator: BinaryOperator::And,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expression, FlowableError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Some(Token::Eq) => BinaryOperator::Eq,
                Some(Token::NotEq) => BinaryOperator::NotEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expression, FlowableError> {
        let mut left = self.parse_addition()?;
        loop {
            let op = match self.peek() {
                Some(Token::Lt) => BinaryOperator::Lt,
                Some(Token::Gt) => BinaryOperator::Gt,
                Some(Token::LtEq) => BinaryOperator::LtEq,
                Some(Token::GtEq) => BinaryOperator::GtEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_addition()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_addition(&mut self) -> Result<Expression, FlowableError> {
        let mut left = self.parse_multiplication()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinaryOperator::Add,
                Some(Token::Minus) => BinaryOperator::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplication()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplication(&mut self) -> Result<Expression, FlowableError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinaryOperator::Mul,
                Some(Token::Slash) => BinaryOperator::Div,
                Some(Token::Percent) => BinaryOperator::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expression, FlowableError> {
        match self.peek() {
            Some(Token::Not) => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expression::UnaryOp {
                    operator: UnaryOperator::Not,
                    operand: Box::new(operand),
                })
            }
            Some(Token::Minus) => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expression::UnaryOp {
                    operator: UnaryOperator::Negate,
                    operand: Box::new(operand),
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expression, FlowableError> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.advance();
                    let property = self.expect_identifier()?;
                    // Check for method call: obj.method(args)
                    if self.peek() == Some(&Token::LeftParen) {
                        self.advance(); // skip (
                        let arguments = self.parse_arguments()?;
                        self.expect(&Token::RightParen)?;
                        expr = Expression::FunctionCall {
                            callee: Box::new(Expression::PropertyAccess {
                                object: Box::new(expr),
                                property,
                            }),
                            arguments,
                        };
                    } else {
                        expr = Expression::PropertyAccess {
                            object: Box::new(expr),
                            property,
                        };
                    }
                }
                Some(Token::LeftBracket) => {
                    self.advance();
                    let index = self.parse_expression()?;
                    self.expect(&Token::RightBracket)?;
                    expr = Expression::IndexAccess {
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                Some(Token::LeftParen) => {
                    self.advance();
                    let arguments = self.parse_arguments()?;
                    self.expect(&Token::RightParen)?;
                    expr = Expression::FunctionCall {
                        callee: Box::new(expr),
                        arguments,
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expression, FlowableError> {
        match self.advance().cloned() {
            Some(Token::Number(n)) => {
                if n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
                    Ok(Expression::Literal(Value::Number((n as i64).into())))
                } else if let Some(num) = serde_json::Number::from_f64(n) {
                    Ok(Expression::Literal(Value::Number(num)))
                } else {
                    Err(FlowableError::ExecutionError(format!(
                        "Invalid numeric literal: {}",
                        n
                    )))
                }
            }
            Some(Token::StringLiteral(s)) => Ok(Expression::Literal(Value::String(s))),
            Some(Token::BoolLiteral(b)) => Ok(Expression::Literal(Value::Bool(b))),
            Some(Token::Null) => Ok(Expression::Literal(Value::Null)),
            Some(Token::Identifier(name)) => Ok(Expression::Variable(name)),
            Some(Token::LeftParen) => {
                let expr = self.parse_expression()?;
                self.expect(&Token::RightParen)?;
                Ok(expr)
            }
            Some(Token::LeftBracket) => {
                // Array literal
                let mut elements = Vec::new();
                if self.peek() != Some(&Token::RightBracket) {
                    elements.push(self.parse_expression()?);
                    while self.peek() == Some(&Token::Comma) {
                        self.advance();
                        if self.peek() == Some(&Token::RightBracket) {
                            break; // trailing comma
                        }
                        elements.push(self.parse_expression()?);
                    }
                }
                self.expect(&Token::RightBracket)?;
                Ok(Expression::ArrayLiteral(elements))
            }
            Some(Token::LeftBrace) => {
                // Object literal
                let mut entries = Vec::new();
                if self.peek() != Some(&Token::RightBrace) {
                    let key = self.parse_object_key()?;
                    self.expect(&Token::Colon)?;
                    let value = self.parse_expression()?;
                    entries.push((key, value));
                    while self.peek() == Some(&Token::Comma) {
                        self.advance();
                        if self.peek() == Some(&Token::RightBrace) {
                            break;
                        }
                        let key = self.parse_object_key()?;
                        self.expect(&Token::Colon)?;
                        let value = self.parse_expression()?;
                        entries.push((key, value));
                    }
                }
                self.expect(&Token::RightBrace)?;
                Ok(Expression::ObjectLiteral(entries))
            }
            Some(other) => Err(FlowableError::ExecutionError(format!(
                "Unexpected token: {:?}",
                other
            ))),
            None => Err(FlowableError::ExecutionError(
                "Unexpected end of script".to_string(),
            )),
        }
    }

    fn parse_arguments(&mut self) -> Result<Vec<Expression>, FlowableError> {
        let mut args = Vec::new();
        if self.peek() != Some(&Token::RightParen) {
            args.push(self.parse_expression()?);
            while self.peek() == Some(&Token::Comma) {
                self.advance();
                args.push(self.parse_expression()?);
            }
        }
        Ok(args)
    }

    fn parse_object_key(&mut self) -> Result<String, FlowableError> {
        match self.advance().cloned() {
            Some(Token::Identifier(name)) => Ok(name),
            Some(Token::StringLiteral(s)) => Ok(s),
            Some(other) => Err(FlowableError::ExecutionError(format!(
                "Expected object key, found {:?}",
                other
            ))),
            None => Err(FlowableError::ExecutionError(
                "Expected object key, found end of script".to_string(),
            )),
        }
    }
}

/// Convenience function: tokenize + parse in one call.
pub fn parse_script(script: &str) -> Result<Vec<Statement>, FlowableError> {
    let tokens = crate::scripting::tokenizer::tokenize(script)?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}
