use super::ast::{BinaryOp, Expr, UnaryOp};
use super::token::{Token, TokenKind};
use crate::error::DmnError;

pub fn parse(tokens: Vec<Token>) -> Result<Expr, DmnError> {
    let mut parser = Parser {
        tokens,
        position: 0,
    };
    let expression = parser.expression()?;
    if !matches!(parser.current().kind, TokenKind::Eof) {
        return Err(parser.error("unexpected token after expression"));
    }
    Ok(expression)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn expression(&mut self) -> Result<Expr, DmnError> {
        match self.current().kind {
            TokenKind::If => self.if_expression(),
            TokenKind::For => self.for_expression(),
            TokenKind::Some | TokenKind::Every => self.quantified_expression(),
            _ => self.or(),
        }
    }

    fn if_expression(&mut self) -> Result<Expr, DmnError> {
        self.bump();
        let condition = self.expression()?;
        self.expect(|kind| matches!(kind, TokenKind::Then), "expected 'then'")?;
        let then_expr = self.expression()?;
        self.expect(|kind| matches!(kind, TokenKind::Else), "expected 'else'")?;
        let else_expr = self.expression()?;
        Ok(Expr::If {
            condition: Box::new(condition),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
        })
    }

    fn for_expression(&mut self) -> Result<Expr, DmnError> {
        self.bump();
        let variable = self.name()?;
        self.expect(|kind| matches!(kind, TokenKind::In), "expected 'in'")?;
        let input = self.or()?;
        self.expect(
            |kind| matches!(kind, TokenKind::Return),
            "expected 'return'",
        )?;
        let body = self.expression()?;
        Ok(Expr::For {
            variable,
            input: Box::new(input),
            body: Box::new(body),
        })
    }

    fn quantified_expression(&mut self) -> Result<Expr, DmnError> {
        let every = matches!(self.current().kind, TokenKind::Every);
        self.bump();
        let variable = self.name()?;
        self.expect(|kind| matches!(kind, TokenKind::In), "expected 'in'")?;
        let input = self.or()?;
        self.expect(
            |kind| matches!(kind, TokenKind::Satisfies),
            "expected 'satisfies'",
        )?;
        let predicate = self.expression()?;
        Ok(Expr::Quantified {
            every,
            variable,
            input: Box::new(input),
            predicate: Box::new(predicate),
        })
    }

    fn or(&mut self) -> Result<Expr, DmnError> {
        self.binary(Self::and, |kind| match kind {
            TokenKind::Or => Some(BinaryOp::Or),
            _ => None,
        })
    }
    fn and(&mut self) -> Result<Expr, DmnError> {
        self.binary(Self::comparison, |kind| match kind {
            TokenKind::And => Some(BinaryOp::And),
            _ => None,
        })
    }
    fn comparison(&mut self) -> Result<Expr, DmnError> {
        self.binary(Self::additive, |kind| match kind {
            TokenKind::Equal => Some(BinaryOp::Equal),
            TokenKind::NotEqual => Some(BinaryOp::NotEqual),
            TokenKind::Less => Some(BinaryOp::Less),
            TokenKind::LessEqual => Some(BinaryOp::LessEqual),
            TokenKind::Greater => Some(BinaryOp::Greater),
            TokenKind::GreaterEqual => Some(BinaryOp::GreaterEqual),
            TokenKind::In => Some(BinaryOp::In),
            _ => None,
        })
    }
    fn additive(&mut self) -> Result<Expr, DmnError> {
        self.binary(Self::multiplicative, |kind| match kind {
            TokenKind::Plus => Some(BinaryOp::Add),
            TokenKind::Minus => Some(BinaryOp::Subtract),
            _ => None,
        })
    }
    fn multiplicative(&mut self) -> Result<Expr, DmnError> {
        self.binary(Self::power, |kind| match kind {
            TokenKind::Star => Some(BinaryOp::Multiply),
            TokenKind::Slash => Some(BinaryOp::Divide),
            _ => None,
        })
    }
    fn power(&mut self) -> Result<Expr, DmnError> {
        self.binary(Self::unary, |kind| match kind {
            TokenKind::Power => Some(BinaryOp::Power),
            _ => None,
        })
    }

    fn binary(
        &mut self,
        operand: fn(&mut Self) -> Result<Expr, DmnError>,
        operator: fn(&TokenKind) -> Option<BinaryOp>,
    ) -> Result<Expr, DmnError> {
        let mut left = operand(self)?;
        while let Some(op) = operator(&self.current().kind) {
            self.bump();
            let right = operand(self)?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, DmnError> {
        if matches!(self.current().kind, TokenKind::Minus) {
            self.bump();
            return Ok(Expr::Unary {
                op: UnaryOp::Negate,
                expr: Box::new(self.unary()?),
            });
        }
        if matches!(self.current().kind, TokenKind::Not) {
            self.bump();
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.unary()?),
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, DmnError> {
        let mut expression = self.primary()?;
        loop {
            if matches!(self.current().kind, TokenKind::Dot) {
                self.bump();
                let key = self.name()?;
                expression = Expr::Path {
                    target: Box::new(expression),
                    key,
                };
                continue;
            }
            if matches!(self.current().kind, TokenKind::LBracket) {
                self.bump();
                let predicate = self.expression()?;
                self.expect(|kind| matches!(kind, TokenKind::RBracket), "expected ']'")?;
                expression = Expr::Filter {
                    target: Box::new(expression),
                    predicate: Box::new(predicate),
                };
                continue;
            }
            break;
        }
        Ok(expression)
    }

    fn primary(&mut self) -> Result<Expr, DmnError> {
        let kind = self.current().kind.clone();
        match kind {
            TokenKind::Null => {
                self.bump();
                Ok(Expr::Null)
            }
            TokenKind::True => {
                self.bump();
                Ok(Expr::Bool(true))
            }
            TokenKind::False => {
                self.bump();
                Ok(Expr::Bool(false))
            }
            TokenKind::Number(value) => {
                self.bump();
                Ok(Expr::Number(value))
            }
            TokenKind::String(value) => {
                self.bump();
                Ok(Expr::String(value))
            }
            TokenKind::Name(name) => {
                self.bump();
                if matches!(self.current().kind, TokenKind::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.current().kind, TokenKind::RParen) {
                        loop {
                            args.push(self.expression()?);
                            if !matches!(self.current().kind, TokenKind::Comma) {
                                break;
                            }
                            self.bump();
                        }
                    }
                    self.expect(|kind| matches!(kind, TokenKind::RParen), "expected ')'")?;
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Name(name))
                }
            }
            TokenKind::LParen => {
                self.bump();
                let start = self.expression()?;
                if matches!(self.current().kind, TokenKind::DotDot) {
                    self.bump();
                    let end = self.expression()?;
                    self.expect(
                        |kind| matches!(kind, TokenKind::RParen | TokenKind::RBracket),
                        "expected range terminator",
                    )?;
                    Ok(Expr::Range {
                        start: Box::new(start),
                        end: Box::new(end),
                        start_inclusive: false,
                        end_inclusive: matches!(self.previous().kind, TokenKind::RBracket),
                    })
                } else {
                    self.expect(|kind| matches!(kind, TokenKind::RParen), "expected ')'")?;
                    Ok(start)
                }
            }
            TokenKind::LBracket => self.list_or_range(),
            TokenKind::LBrace => self.context(),
            _ => Err(self.error("expected FEEL expression")),
        }
    }

    fn list_or_range(&mut self) -> Result<Expr, DmnError> {
        self.bump();
        if matches!(self.current().kind, TokenKind::RBracket) {
            self.bump();
            return Ok(Expr::List(Vec::new()));
        }
        let first = self.expression()?;
        if matches!(self.current().kind, TokenKind::DotDot) {
            self.bump();
            let end = self.expression()?;
            let inclusive = matches!(self.current().kind, TokenKind::RBracket);
            self.expect(
                |kind| matches!(kind, TokenKind::RBracket | TokenKind::RParen),
                "expected range terminator",
            )?;
            return Ok(Expr::Range {
                start: Box::new(first),
                end: Box::new(end),
                start_inclusive: true,
                end_inclusive: inclusive,
            });
        }
        let mut values = vec![first];
        while matches!(self.current().kind, TokenKind::Comma) {
            self.bump();
            values.push(self.expression()?);
        }
        self.expect(|kind| matches!(kind, TokenKind::RBracket), "expected ']'")?;
        Ok(Expr::List(values))
    }

    fn context(&mut self) -> Result<Expr, DmnError> {
        self.bump();
        let mut entries = Vec::new();
        if !matches!(self.current().kind, TokenKind::RBrace) {
            loop {
                let key = match self.current().kind.clone() {
                    TokenKind::Name(name) | TokenKind::String(name) => {
                        self.bump();
                        name
                    }
                    _ => return Err(self.error("expected context key")),
                };
                self.expect(|kind| matches!(kind, TokenKind::Colon), "expected ':'")?;
                entries.push((key, self.expression()?));
                if !matches!(self.current().kind, TokenKind::Comma) {
                    break;
                }
                self.bump();
            }
        }
        self.expect(|kind| matches!(kind, TokenKind::RBrace), "expected '}'")?;
        Ok(Expr::Context(entries))
    }

    fn name(&mut self) -> Result<String, DmnError> {
        match self.current().kind.clone() {
            TokenKind::Name(name) => {
                self.bump();
                Ok(name)
            }
            _ => Err(self.error("expected name")),
        }
    }
    fn expect(
        &mut self,
        predicate: impl FnOnce(&TokenKind) -> bool,
        message: &str,
    ) -> Result<(), DmnError> {
        if predicate(&self.current().kind) {
            self.bump();
            Ok(())
        } else {
            Err(self.error(message))
        }
    }
    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }
    fn previous(&self) -> &Token {
        &self.tokens[self.position.saturating_sub(1)]
    }
    fn bump(&mut self) {
        if self.position + 1 < self.tokens.len() {
            self.position += 1;
        }
    }
    fn error(&self, message: &str) -> DmnError {
        DmnError::validation(format!("{message} at byte {}", self.current().start))
    }
}
