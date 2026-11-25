//! Extracts chartable numbers from payloads.
//!
//! A chart series is identified by a topic and a JSON Pointer (RFC 6901) into the decoded payload; the empty
//! pointer means "the payload itself".

use serde_json::Value;

use crate::decode::{Body, Decoded};

/// Parses a plain-text payload as a number. Booleans and the usual on/off words count as 1 and 0.
#[must_use]
pub fn parse_number(text: &str) -> Option<f64> {
    let text = text.trim();
    if let Ok(number) = text.parse::<f64>() {
        return number.is_finite().then_some(number);
    }
    match text.to_ascii_lowercase().as_str() {
        "true" | "on" => Some(1.0),
        "false" | "off" => Some(0.0),
        _ => None,
    }
}

/// The number a JSON value stands for, if any.
#[must_use]
pub fn value_as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::Bool(boolean) => Some(f64::from(u8::from(*boolean))),
        Value::String(text) => parse_number(text),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// The number at `pointer` inside a decoded payload.
#[must_use]
pub fn extract(decoded: &Decoded, pointer: &str) -> Option<f64> {
    match &decoded.body {
        Body::Text(text) if pointer.is_empty() => parse_number(text),
        Body::Json(value) => value.pointer(pointer).and_then(value_as_number),
        Body::Text(_) | Body::Hex(_) | Body::Error(_) => None,
    }
}

/// JSON Pointers to every numeric leaf of `value`, depth first, at most `limit` of them.
#[must_use]
pub fn numeric_paths(value: &Value, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    collect(value, &mut String::new(), limit, &mut out);
    out
}

fn collect(value: &Value, path: &mut String, limit: usize, out: &mut Vec<String>) {
    if out.len() >= limit {
        return;
    }
    let base = path.len();
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                path.push('/');
                path.push_str(&key.replace('~', "~0").replace('/', "~1"));
                collect(child, path, limit, out);
                path.truncate(base);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                path.push('/');
                path.push_str(&index.to_string());
                collect(child, path, limit, out);
                path.truncate(base);
            }
        }
        Value::Number(_) | Value::Bool(_) => out.push(path.clone()),
        Value::String(text) => {
            if text.trim().parse::<f64>().is_ok() {
                out.push(path.clone());
            }
        }
        Value::Null => {}
    }
}

/// Whether a decoded payload has anything to chart, and where.
#[must_use]
pub fn chartable_paths(decoded: &Decoded, limit: usize) -> Vec<String> {
    match &decoded.body {
        Body::Text(text) => parse_number(text).map_or_else(Vec::new, |_| vec![String::new()]),
        Body::Json(value) => numeric_paths(value, limit),
        Body::Hex(_) | Body::Error(_) => Vec::new(),
    }
}
