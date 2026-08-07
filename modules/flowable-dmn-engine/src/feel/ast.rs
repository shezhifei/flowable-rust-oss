#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Name(String),
    List(Vec<Expr>),
    Context(Vec<(String, Expr)>),
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        start_inclusive: bool,
        end_inclusive: bool,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    If {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    Path {
        target: Box<Expr>,
        key: String,
    },
    Filter {
        target: Box<Expr>,
        predicate: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    For {
        variable: String,
        input: Box<Expr>,
        body: Box<Expr>,
    },
    Quantified {
        every: bool,
        variable: String,
        input: Box<Expr>,
        predicate: Box<Expr>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    And,
    Or,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    In,
}
