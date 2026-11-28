//! Turns raw payload bytes into something readable.
//!
//! Every structured decoder produces a [`serde_json::Value`], so the value view, the diff and the charts share
//! one code path regardless of the wire format.

mod json;
mod proto;

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use self::proto::ProtoRegistry;

/// A payload interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Pick a format by looking at the topic and the bytes.
    #[default]
    Auto,
    /// UTF-8 text.
    Text,
    /// JSON.
    Json,
    /// Hex dump.
    Hex,
    /// Base64 text wrapping another payload.
    Base64,
    /// `MessagePack`.
    MsgPack,
    /// CBOR.
    Cbor,
    /// Protobuf, using the message type bound to the topic.
    Protobuf,
    /// Sparkplug B.
    Sparkplug,
}

impl Format {
    /// All formats, in display order.
    pub const ALL: [Self; 9] = [
        Self::Auto,
        Self::Text,
        Self::Json,
        Self::Hex,
        Self::Base64,
        Self::MsgPack,
        Self::Cbor,
        Self::Protobuf,
        Self::Sparkplug,
    ];

    /// Label shown in the UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Text => "Text",
            Self::Json => "JSON",
            Self::Hex => "Hex",
            Self::Base64 => "Base64",
            Self::MsgPack => "MessagePack",
            Self::Cbor => "CBOR",
            Self::Protobuf => "Protobuf",
            Self::Sparkplug => "Sparkplug B",
        }
    }
}

/// The decoded content of a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// Plain text.
    Text(String),
    /// Structured data.
    Json(Value),
    /// A hex dump of bytes with no better interpretation.
    Hex(String),
    /// The payload could not be decoded in the requested format.
    Error(String),
}

/// A payload after decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The format that was actually applied (never [`Format::Auto`]).
    pub format: Format,
    /// The result.
    pub body: Body,
    /// The JSON text as received, when the payload was JSON text. Shown instead of the parsed value, which
    /// sorts out duplicate keys and rewrites numbers.
    source: Option<String>,
}

impl Decoded {
    /// The text to show: pretty-printed JSON, the text itself, the hex dump or the error.
    #[must_use]
    pub fn display(&self) -> String {
        match &self.body {
            Body::Text(text) | Body::Hex(text) => text.clone(),
            Body::Json(value) => self.source.as_deref().map_or_else(
                || serde_json::to_string_pretty(value).unwrap_or_else(|error| error.to_string()),
                json::pretty,
            ),
            Body::Error(error) => format!("Cannot decode as {}: {error}", self.format.label()),
        }
    }

    /// The structured value, if the payload decoded to one.
    #[must_use]
    pub const fn json(&self) -> Option<&Value> {
        match &self.body {
            Body::Json(value) => Some(value),
            Body::Text(_) | Body::Hex(_) | Body::Error(_) => None,
        }
    }

    /// Whether decoding failed.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self.body, Body::Error(_))
    }
}

/// Decodes payloads; holds the loaded Protobuf schemas.
#[derive(Debug, Clone, Default)]
pub struct Decoder {
    proto: ProtoRegistry,
}

impl Decoder {
    /// A decoder with the built-in schemas only.
    #[must_use]
    pub fn new() -> Self {
        Self { proto: ProtoRegistry::new() }
    }

    /// A decoder using `proto` for Protobuf payloads.
    #[must_use]
    pub const fn with_registry(proto: ProtoRegistry) -> Self {
        Self { proto }
    }

    /// The Protobuf registry in use.
    #[must_use]
    pub const fn registry(&self) -> &ProtoRegistry {
        &self.proto
    }

    /// The format [`Format::Auto`] resolves to for this topic and payload.
    #[must_use]
    pub fn detect(&self, topic: &str, payload: &[u8]) -> Format {
        if topic.starts_with("spBv1.0/") && self.proto.decode_sparkplug(payload).is_ok() {
            return Format::Sparkplug;
        }
        if self.proto.binding_for(topic).is_some() {
            return Format::Protobuf;
        }
        match std::str::from_utf8(payload) {
            Ok(text) if looks_like_json(text) && serde_json::from_str::<Value>(text).is_ok() => Format::Json,
            Ok(text) if is_printable(text) => Format::Text,
            Ok(_) | Err(_) => Format::Hex,
        }
    }

    /// Decodes `payload` as `format`. Never panics; undecodable input yields [`Body::Error`].
    #[must_use]
    pub fn decode(&self, topic: &str, payload: &[u8], format: Format) -> Decoded {
        let format = if format == Format::Auto { self.detect(topic, payload) } else { format };
        let mut source = None;
        let body = match format {
            Format::Auto | Format::Text => match std::str::from_utf8(payload) {
                Ok(text) => Body::Text(text.to_owned()),
                Err(error) => Body::Error(format!("not valid UTF-8: {error}")),
            },
            Format::Json => {
                let body = json_body(serde_json::from_slice::<Value>(payload).map_err(|error| error.to_string()));
                // Parsing succeeded, so the bytes are UTF-8.
                source = matches!(body, Body::Json(_)).then(|| String::from_utf8_lossy(payload).into_owned());
                body
            }
            Format::Hex => Body::Hex(hex_dump(payload)),
            Format::Base64 => return self.decode_base64(topic, payload),
            Format::MsgPack => json_body(decode_msgpack(payload)),
            Format::Cbor => json_body(decode_cbor(payload)),
            Format::Protobuf => json_body(self.proto.decode(topic, payload)),
            Format::Sparkplug => json_body(self.proto.decode_sparkplug(payload)),
        };
        Decoded { format, body, source }
    }

    fn decode_base64(&self, topic: &str, payload: &[u8]) -> Decoded {
        let trimmed = payload.trim_ascii();
        let (body, source) = match STANDARD.decode(trimmed) {
            Ok(inner) => {
                // Only content sniffing here: the topic's own binding describes the outer payload.
                let inner_format = match std::str::from_utf8(&inner) {
                    Ok(text) if looks_like_json(text) && serde_json::from_str::<Value>(text).is_ok() => Format::Json,
                    Ok(text) if is_printable(text) => Format::Text,
                    Ok(_) | Err(_) => Format::Hex,
                };
                let decoded = self.decode(topic, &inner, inner_format);
                (decoded.body, decoded.source)
            }
            Err(error) => (Body::Error(error.to_string()), None),
        };
        Decoded { format: Format::Base64, body, source }
    }
}

fn json_body(result: Result<Value, String>) -> Body {
    match result {
        Ok(value) => Body::Json(value),
        Err(error) => Body::Error(error),
    }
}

fn looks_like_json(text: &str) -> bool {
    // Bare numbers, strings and literals stay text: "21.5" reads better as a value than as a JSON document.
    matches!(text.trim_start().as_bytes().first(), Some(b'{' | b'['))
}

fn is_printable(text: &str) -> bool {
    !text.chars().any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

/// A classic 16-bytes-per-line hex dump with offsets and an ASCII column.
#[must_use]
pub fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 + 16);
    for (index, chunk) in bytes.chunks(16).enumerate() {
        let _ = write!(out, "{:08x} ", index * 16);
        for position in 0..16 {
            if position == 8 {
                out.push(' ');
            }
            match chunk.get(position) {
                Some(byte) => {
                    let _ = write!(out, " {byte:02x}");
                }
                None => out.push_str("   "),
            }
        }
        out.push_str("  |");
        out.extend(
            chunk.iter().map(|byte| if byte.is_ascii_graphic() || *byte == b' ' { char::from(*byte) } else { '.' }),
        );
        out.push_str("|\n");
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Parses text written as hex (whitespace and `0x` prefixes allowed) into bytes.
///
/// # Errors
/// Fails on non-hex characters or an odd number of digits.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = text
        .split_whitespace()
        .flat_map(|word| word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")).unwrap_or(word).bytes())
        .collect();
    let (pairs, rest) = digits.as_chunks::<2>();
    if !rest.is_empty() {
        return Err("hex input needs an even number of digits".to_owned());
    }
    pairs
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(text, 16).map_err(|_| format!("'{text}' is not a hex byte"))
        })
        .collect()
}

/// A short single-line rendering of a payload for the tree.
#[must_use]
pub fn preview(payload: &[u8], max_chars: usize) -> String {
    if payload.is_empty() {
        return String::new();
    }
    // Only look at a prefix: previews are drawn for every visible row on every frame.
    let window = &payload[..payload.len().min(max_chars.saturating_mul(4))];
    let text = match std::str::from_utf8(window) {
        Ok(text) => text,
        // A multi-byte character cut by the window is still text.
        Err(error) if error.error_len().is_none() => std::str::from_utf8(&window[..error.valid_up_to()]).unwrap_or(""),
        Err(_) => "",
    };
    if text.is_empty() || !is_printable(text) {
        let mut out = String::new();
        for byte in payload.iter().take(max_chars / 3) {
            let _ = write!(out, "{byte:02x} ");
        }
        return format!("{}({} bytes)", out, payload.len());
    }
    let mut out = String::with_capacity(max_chars);
    let mut count = 0;
    let mut last_space = false;
    for character in text.chars() {
        let character = if character.is_whitespace() { ' ' } else { character };
        if character == ' ' && last_space {
            continue;
        }
        if count == max_chars {
            out.push('…');
            return out;
        }
        last_space = character == ' ';
        out.push(character);
        count += 1;
    }
    if window.len() < payload.len() {
        out.push('…');
    }
    out
}

fn decode_msgpack(payload: &[u8]) -> Result<Value, String> {
    let mut reader = payload;
    let value = rmpv::decode::read_value(&mut reader).map_err(|error| error.to_string())?;
    if !reader.is_empty() {
        return Err(format!("{} trailing bytes after the MessagePack value", reader.len()));
    }
    Ok(msgpack_to_json(value))
}

fn msgpack_to_json(value: rmpv::Value) -> Value {
    match value {
        rmpv::Value::Nil => Value::Null,
        rmpv::Value::Boolean(boolean) => Value::Bool(boolean),
        rmpv::Value::Integer(integer) => {
            integer.as_u64().map(Value::from).or_else(|| integer.as_i64().map(Value::from)).unwrap_or(Value::Null)
        }
        rmpv::Value::F32(float) => float_to_json(f64::from(float)),
        rmpv::Value::F64(float) => float_to_json(float),
        rmpv::Value::String(text) => text.into_str().map_or(Value::Null, Value::String),
        rmpv::Value::Binary(bytes) => Value::String(STANDARD.encode(bytes)),
        rmpv::Value::Array(items) => Value::Array(items.into_iter().map(msgpack_to_json).collect()),
        rmpv::Value::Map(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| {
                    let key = if let rmpv::Value::String(text) = key {
                        text.into_str().unwrap_or_default()
                    } else {
                        key.to_string()
                    };
                    (key, msgpack_to_json(value))
                })
                .collect(),
        ),
        rmpv::Value::Ext(kind, bytes) => {
            serde_json::json!({ "ext_type": kind, "data": STANDARD.encode(bytes) })
        }
    }
}

fn decode_cbor(payload: &[u8]) -> Result<Value, String> {
    let mut reader = payload;
    let value: ciborium::Value = ciborium::from_reader(&mut reader).map_err(|error| error.to_string())?;
    if !reader.is_empty() {
        return Err(format!("{} trailing bytes after the CBOR value", reader.len()));
    }
    Ok(cbor_to_json(value))
}

#[expect(clippy::wildcard_enum_match_arm, reason = "`ciborium::Value` is non-exhaustive")]
fn cbor_to_json(value: ciborium::Value) -> Value {
    match value {
        ciborium::Value::Integer(integer) => {
            let wide = i128::from(integer);
            u64::try_from(wide)
                .map(Value::from)
                .or_else(|_| i64::try_from(wide).map(Value::from))
                .unwrap_or(Value::Null)
        }
        ciborium::Value::Bytes(bytes) => Value::String(STANDARD.encode(bytes)),
        ciborium::Value::Float(float) => float_to_json(float),
        ciborium::Value::Text(text) => Value::String(text),
        ciborium::Value::Bool(boolean) => Value::Bool(boolean),
        ciborium::Value::Tag(_, inner) => cbor_to_json(*inner),
        ciborium::Value::Array(items) => Value::Array(items.into_iter().map(cbor_to_json).collect()),
        ciborium::Value::Map(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| {
                    let key = match cbor_to_json(key) {
                        Value::String(text) => text,
                        json @ (Value::Null
                        | Value::Bool(_)
                        | Value::Number(_)
                        | Value::Array(_)
                        | Value::Object(_)) => json.to_string(),
                    };
                    (key, cbor_to_json(value))
                })
                .collect(),
        ),
        // `ciborium::Value` is non-exhaustive; null and anything it adds later have no JSON counterpart.
        _ => Value::Null,
    }
}

fn float_to_json(float: f64) -> Value {
    serde_json::Number::from_f64(float).map_or_else(|| Value::String(float.to_string()), Value::Number)
}
