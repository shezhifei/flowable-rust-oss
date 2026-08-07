use crate::error::FlowableError;

/// Token types produced by the script lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Number(f64),
    StringLiteral(String),
    BoolLiteral(bool),
    Null,

    // Identifiers and keywords
    Identifier(String),
    Var,
    Let,
    If,
    Else,
    For,
    While,
    Function,
    Return,

    // Arithmetic operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    // Assignment
    Assign,
    PlusAssign,
    MinusAssign,

    // Comparison operators
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,

    // Logical operators
    And,
    Or,
    Not,

    // Delimiters
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Semicolon,
    Dot,
    Colon,
}

/// Tokenize a script string into a list of tokens.
pub fn tokenize(script: &str) -> Result<Vec<Token>, FlowableError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = script.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        // Skip whitespace
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Skip line comments
        if ch == '/' && i + 1 < len && chars[i + 1] == '/' {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Skip block comments
        if ch == '/' && i + 1 < len && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2; // skip */
            continue;
        }

        // String literals
        if ch == '\'' || ch == '"' {
            let quote = ch;
            i += 1;
            let mut s = String::new();
            while i < len && chars[i] != quote {
                if chars[i] == '\\' && i + 1 < len {
                    i += 1;
                    match chars[i] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        c if c == quote => s.push(c),
                        c => {
                            s.push('\\');
                            s.push(c);
                        }
                    }
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            if i >= len {
                return Err(FlowableError::ExecutionError(
                    "Unterminated string literal".to_string(),
                ));
            }
            i += 1; // skip closing quote
            tokens.push(Token::StringLiteral(s));
            continue;
        }

        // Numbers
        if ch.is_ascii_digit() || (ch == '.' && i + 1 < len && chars[i + 1].is_ascii_digit()) {
            let start = i;
            while i < len && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let num_str: String = chars[start..i].iter().collect();
            let num = num_str.parse::<f64>().map_err(|_| {
                FlowableError::ExecutionError(format!("Invalid number: {}", num_str))
            })?;
            tokens.push(Token::Number(num));
            continue;
        }

        // Identifiers and keywords
        if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' {
            let start = i;
            while i < len
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let token = match word.as_str() {
                "var" => Token::Var,
                "let" => Token::Let,
                "if" => Token::If,
                "else" => Token::Else,
                "for" => Token::For,
                "while" => Token::While,
                "function" => Token::Function,
                "return" => Token::Return,
                "true" => Token::BoolLiteral(true),
                "false" => Token::BoolLiteral(false),
                "null" | "undefined" => Token::Null,
                "def" => Token::Var, // Groovy declaration token
                _ => Token::Identifier(word),
            };
            tokens.push(token);
            continue;
        }

        // Two-character operators
        if i + 1 < len {
            let two: String = chars[i..i + 2].iter().collect();
            let matched = match two.as_str() {
                "==" => Some(Token::Eq),
                "!=" => Some(Token::NotEq),
                "<=" => Some(Token::LtEq),
                ">=" => Some(Token::GtEq),
                "&&" => Some(Token::And),
                "||" => Some(Token::Or),
                "+=" => Some(Token::PlusAssign),
                "-=" => Some(Token::MinusAssign),
                _ => None,
            };
            if let Some(token) = matched {
                tokens.push(token);
                i += 2;
                continue;
            }
        }

        // Single-character operators and delimiters
        let token = match ch {
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' => Token::Star,
            '/' => Token::Slash,
            '%' => Token::Percent,
            '=' => Token::Assign,
            '<' => Token::Lt,
            '>' => Token::Gt,
            '!' => Token::Not,
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            '{' => Token::LeftBrace,
            '}' => Token::RightBrace,
            '[' => Token::LeftBracket,
            ']' => Token::RightBracket,
            ',' => Token::Comma,
            ';' => Token::Semicolon,
            '.' => Token::Dot,
            ':' => Token::Colon,
            _ => {
                return Err(FlowableError::ExecutionError(format!(
                    "Unexpected character in script: '{}'",
                    ch
                )));
            }
        };
        tokens.push(token);
        i += 1;
    }

    Ok(tokens)
}
