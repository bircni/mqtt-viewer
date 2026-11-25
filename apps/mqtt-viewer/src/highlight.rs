//! A tiny JSON tokenizer for syntax colouring.
//!
//! It never fails: text that is not JSON simply ends up as [`Token::Other`], so it can run on anything the
//! value view shows.

use std::ops::Range;

/// The kind of a piece of JSON text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// An object key, including its quotes.
    Key,
    /// A string value, including its quotes.
    String,
    /// A number.
    Number,
    /// `true`, `false` or `null`.
    Literal,
    /// Braces, brackets, colons and commas.
    Punctuation,
    /// Whitespace and anything unrecognised.
    Other,
}

/// Splits `text` into consecutive tokens covering every byte.
#[must_use]
pub fn json_tokens(text: &str) -> Vec<(Token, Range<usize>)> {
    let bytes = text.as_bytes();
    let mut tokens: Vec<(Token, Range<usize>)> = Vec::new();
    let mut push = |token: Token, range: Range<usize>| match tokens.last_mut() {
        Some((last, last_range)) if *last == token && token == Token::Other => last_range.end = range.end,
        _ => tokens.push((token, range)),
    };
    let mut position = 0;
    while position < bytes.len() {
        let start = position;
        match bytes[position] {
            b'"' => {
                position += 1;
                while position < bytes.len() && bytes[position] != b'"' {
                    position += if bytes[position] == b'\\' { 2 } else { 1 };
                }
                position = (position + 1).min(bytes.len());
                let rest = &bytes[position..];
                let is_key = rest.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b':');
                push(if is_key { Token::Key } else { Token::String }, start..position);
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                position += 1;
                push(Token::Punctuation, start..position);
            }
            b'-' | b'0'..=b'9' => {
                while position < bytes.len()
                    && matches!(bytes[position], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    position += 1;
                }
                push(Token::Number, start..position);
            }
            _ => {
                let literal =
                    ["true", "false", "null"].into_iter().find(|literal| text[position..].starts_with(literal));
                if let Some(literal) = literal {
                    position += literal.len();
                    push(Token::Literal, start..position);
                } else {
                    // Advance one whole character so ranges stay on UTF-8 boundaries.
                    position += text[position..].chars().next().map_or(1, char::len_utf8);
                    push(Token::Other, start..position);
                }
            }
        }
    }
    tokens
}
