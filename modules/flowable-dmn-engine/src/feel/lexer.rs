use super::token::{Token, TokenKind};
use crate::error::DmnError;

pub fn lex(source: &str) -> Result<Vec<Token>, DmnError> {
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let (start, ch) = chars[index];
        if ch.is_whitespace() {
            index += 1;
            continue;
        }
        let single = |kind, end| Token { kind, start, end };
        match ch {
            '(' => {
                tokens.push(single(TokenKind::LParen, start + 1));
                index += 1;
            }
            ')' => {
                tokens.push(single(TokenKind::RParen, start + 1));
                index += 1;
            }
            '[' => {
                tokens.push(single(TokenKind::LBracket, start + 1));
                index += 1;
            }
            ']' => {
                tokens.push(single(TokenKind::RBracket, start + 1));
                index += 1;
            }
            '{' => {
                tokens.push(single(TokenKind::LBrace, start + 1));
                index += 1;
            }
            '}' => {
                tokens.push(single(TokenKind::RBrace, start + 1));
                index += 1;
            }
            ',' => {
                tokens.push(single(TokenKind::Comma, start + 1));
                index += 1;
            }
            ':' => {
                tokens.push(single(TokenKind::Colon, start + 1));
                index += 1;
            }
            '+' => {
                tokens.push(single(TokenKind::Plus, start + 1));
                index += 1;
            }
            '-' => {
                tokens.push(single(TokenKind::Minus, start + 1));
                index += 1;
            }
            '*' if chars.get(index + 1).map(|(_, c)| *c) == Some('*') => {
                tokens.push(single(TokenKind::Power, start + 2));
                index += 2;
            }
            '*' => {
                tokens.push(single(TokenKind::Star, start + 1));
                index += 1;
            }
            '/' => {
                tokens.push(single(TokenKind::Slash, start + 1));
                index += 1;
            }
            '.' if chars.get(index + 1).map(|(_, c)| *c) == Some('.') => {
                tokens.push(single(TokenKind::DotDot, start + 2));
                index += 2;
            }
            '.' => {
                tokens.push(single(TokenKind::Dot, start + 1));
                index += 1;
            }
            '=' => {
                tokens.push(single(TokenKind::Equal, start + 1));
                index += 1;
            }
            '!' if chars.get(index + 1).map(|(_, c)| *c) == Some('=') => {
                tokens.push(single(TokenKind::NotEqual, start + 2));
                index += 2;
            }
            '<' if chars.get(index + 1).map(|(_, c)| *c) == Some('=') => {
                tokens.push(single(TokenKind::LessEqual, start + 2));
                index += 2;
            }
            '>' if chars.get(index + 1).map(|(_, c)| *c) == Some('=') => {
                tokens.push(single(TokenKind::GreaterEqual, start + 2));
                index += 2;
            }
            '<' => {
                tokens.push(single(TokenKind::Less, start + 1));
                index += 1;
            }
            '>' => {
                tokens.push(single(TokenKind::Greater, start + 1));
                index += 1;
            }
            quote @ ('"' | '\'') => {
                index += 1;
                let mut value = String::new();
                let mut closed = false;
                while index < chars.len() {
                    let (_, current) = chars[index];
                    if current == quote {
                        closed = true;
                        index += 1;
                        break;
                    }
                    if current == '\\' {
                        index += 1;
                        let escaped = chars.get(index).map(|(_, c)| *c).ok_or_else(|| {
                            DmnError::validation("unterminated FEEL string escape")
                        })?;
                        value.push(match escaped {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            other => other,
                        });
                    } else {
                        value.push(current);
                    }
                    index += 1;
                }
                if !closed {
                    return Err(DmnError::validation(format!(
                        "unterminated FEEL string at byte {start}"
                    )));
                }
                let end = chars
                    .get(index)
                    .map(|(byte, _)| *byte)
                    .unwrap_or(source.len());
                tokens.push(Token {
                    kind: TokenKind::String(value),
                    start,
                    end,
                });
            }
            c if c.is_ascii_digit() => {
                let mut end_index = index + 1;
                while end_index < chars.len()
                    && (chars[end_index].1.is_ascii_digit() || chars[end_index].1 == '.')
                {
                    if chars[end_index].1 == '.'
                        && chars.get(end_index + 1).map(|(_, c)| *c) == Some('.')
                    {
                        break;
                    }
                    end_index += 1;
                }
                let end = chars
                    .get(end_index)
                    .map(|(byte, _)| *byte)
                    .unwrap_or(source.len());
                let value = source[start..end].parse::<f64>().map_err(|_| {
                    DmnError::validation(format!("invalid FEEL number at byte {start}"))
                })?;
                tokens.push(Token {
                    kind: TokenKind::Number(value),
                    start,
                    end,
                });
                index = end_index;
            }
            c if is_name_start(c) => {
                let mut end_index = index + 1;
                while end_index < chars.len() && is_name_continue(chars[end_index].1) {
                    end_index += 1;
                }
                let end = chars
                    .get(end_index)
                    .map(|(byte, _)| *byte)
                    .unwrap_or(source.len());
                let word = &source[start..end];
                let kind = match word.to_ascii_lowercase().as_str() {
                    "null" => TokenKind::Null,
                    "true" => TokenKind::True,
                    "false" => TokenKind::False,
                    "if" => TokenKind::If,
                    "then" => TokenKind::Then,
                    "else" => TokenKind::Else,
                    "for" => TokenKind::For,
                    "return" => TokenKind::Return,
                    "some" => TokenKind::Some,
                    "every" => TokenKind::Every,
                    "satisfies" => TokenKind::Satisfies,
                    "in" => TokenKind::In,
                    "and" => TokenKind::And,
                    "or" => TokenKind::Or,
                    "not" => TokenKind::Not,
                    _ => TokenKind::Name(word.to_string()),
                };
                tokens.push(Token { kind, start, end });
                index = end_index;
            }
            _ => {
                return Err(DmnError::validation(format!(
                    "unexpected FEEL character '{ch}' at byte {start}"
                )));
            }
        }
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        start: source.len(),
        end: source.len(),
    });
    Ok(tokens)
}

fn is_name_start(ch: char) -> bool {
    ch == '_' || ch.is_alphabetic()
}
fn is_name_continue(ch: char) -> bool {
    ch == '_' || ch == '-' || ch.is_alphanumeric()
}
