use serde_json::Value;

/// AST expression nodes for the script engine.
#[derive(Debug, Clone)]
pub enum Expression {
    /// Literal value: number, string, boolean, null
    Literal(Value),

    /// Array literal: [expr, expr, ...]
    ArrayLiteral(Vec<Expression>),

    /// Object literal: { key: expr, key: expr, ... }
    ObjectLiteral(Vec<(String, Expression)>),

    /// Variable reference: `name`
    Variable(String),

    /// Binary operation: left op right
    BinaryOp {
        left: Box<Expression>,
        operator: BinaryOperator,
        right: Box<Expression>,
    },

    /// Unary operation: !expr, -expr
    UnaryOp {
        operator: UnaryOperator,
        operand: Box<Expression>,
    },

    /// Property access: object.property
    PropertyAccess {
        object: Box<Expression>,
        property: String,
    },

    /// Index access: array[index]
    IndexAccess {
        object: Box<Expression>,
        index: Box<Expression>,
    },

    /// Function call: callee(args...)
    FunctionCall {
        callee: Box<Expression>,
        arguments: Vec<Expression>,
    },

    /// Assignment: name = expr
    Assignment {
        name: String,
        value: Box<Expression>,
    },

    /// Compound assignment: name += expr, name -= expr
    CompoundAssignment {
        name: String,
        operator: BinaryOperator,
        value: Box<Expression>,
    },
}

/// Binary operators with known precedence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOperator {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOperator {
    Not,
    Negate,
}

/// AST statement nodes for the script engine.
#[derive(Debug, Clone)]
pub enum Statement {
    /// Variable declaration: var/let name = expr;
    VarDecl {
        name: String,
        initializer: Option<Expression>,
    },

    /// Expression statement: expr;
    ExpressionStmt(Expression),

    /// If statement: if (cond) { ... } else { ... }
    IfStmt {
        condition: Expression,
        then_body: Vec<Statement>,
        else_body: Option<Vec<Statement>>,
    },

    /// For statement: for (init; cond; update) { body }
    ForStmt {
        init: Option<Box<Statement>>,
        condition: Option<Expression>,
        update: Option<Expression>,
        body: Vec<Statement>,
    },

    /// While statement: while (cond) { body }
    WhileStmt {
        condition: Expression,
        body: Vec<Statement>,
    },

    /// Function declaration: function name(params) { body }
    FunctionDecl {
        name: String,
        params: Vec<String>,
        body: Vec<Statement>,
    },

    /// Return statement: return expr;
    ReturnStmt(Option<Expression>),

    /// Block: { statements }
    Block(Vec<Statement>),
}
