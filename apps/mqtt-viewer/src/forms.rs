//! Editable form state behind the publish panel.

use bytes::Bytes;
use mqtt_viewer_core::decode::parse_hex;
use mqtt_viewer_core::message::{Properties, PublishRequest, QoS};
use mqtt_viewer_core::topic;

/// How the payload editor's text is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PayloadMode {
    /// Sent as typed.
    #[default]
    Raw,
    /// Must be valid JSON.
    Json,
    /// Must be well-formed XML.
    Xml,
    /// Hex digits, converted to bytes.
    Hex,
}

impl PayloadMode {
    /// All modes, in display order.
    pub const ALL: [Self; 4] = [Self::Raw, Self::Json, Self::Xml, Self::Hex];

    /// Label shown in the UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Json => "json",
            Self::Xml => "xml",
            Self::Hex => "hex",
        }
    }
}

/// The publish panel's fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PublishForm {
    /// Target topic.
    pub topic: String,
    /// Payload text.
    pub payload: String,
    /// Interpretation of the payload text.
    pub mode: PayloadMode,
    /// QoS to publish with.
    pub qos: QoS,
    /// Retain flag.
    pub retain: bool,
    /// MQTT 5 user properties.
    pub user_properties: Vec<(String, String)>,
    /// MQTT 5 content type.
    pub content_type: String,
    /// MQTT 5 response topic.
    pub response_topic: String,
    /// MQTT 5 correlation data, as text.
    pub correlation_data: String,
    /// MQTT 5 message expiry in seconds, as typed.
    pub expiry_secs: String,
    /// MQTT 5 payload format indicator.
    pub mark_utf8: bool,
}

impl PublishForm {
    /// Checks the payload against the selected mode without building a request.
    ///
    /// # Errors
    /// Returns what is wrong with the payload.
    pub fn check_payload(&self) -> Result<Bytes, String> {
        match self.mode {
            PayloadMode::Raw => Ok(Bytes::from(self.payload.clone())),
            PayloadMode::Json => serde_json::from_str::<serde_json::Value>(&self.payload)
                .map(|_| Bytes::from(self.payload.clone()))
                .map_err(|error| format!("invalid JSON: {error}")),
            PayloadMode::Xml => check_xml(&self.payload).map(|()| Bytes::from(self.payload.clone())),
            PayloadMode::Hex => parse_hex(&self.payload).map(Bytes::from),
        }
    }

    /// Builds the request the form describes.
    ///
    /// # Errors
    /// Returns the first problem with the topic, the payload or the properties.
    pub fn to_request(&self) -> Result<PublishRequest, String> {
        let topic = self.topic.trim();
        topic::validate_topic(topic)?;
        let payload = self.check_payload()?;
        let expiry = self.expiry_secs.trim();
        let message_expiry_secs = if expiry.is_empty() {
            None
        } else {
            Some(
                expiry
                    .parse::<u32>()
                    .map_err(|_| format!("message expiry must be a number of seconds, not '{expiry}'"))?,
            )
        };
        if !self.response_topic.is_empty() {
            topic::validate_topic(&self.response_topic).map_err(|error| format!("response topic: {error}"))?;
        }
        let text = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        let properties = Properties {
            user_properties: self.user_properties.iter().filter(|(key, _)| !key.is_empty()).cloned().collect(),
            content_type: text(&self.content_type),
            response_topic: text(&self.response_topic),
            correlation_data: text(&self.correlation_data).map(Bytes::from),
            message_expiry_secs,
            payload_is_utf8: self.mark_utf8.then_some(true),
        };
        Ok(PublishRequest { topic: topic.to_owned(), payload, qos: self.qos, retain: self.retain, properties })
    }

    /// Pretty-prints the payload when it is valid JSON.
    ///
    /// # Errors
    /// Returns the parse error when it is not.
    pub fn format_json(&mut self) -> Result<(), String> {
        let value: serde_json::Value =
            serde_json::from_str(&self.payload).map_err(|error| format!("invalid JSON: {error}"))?;
        self.payload = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Fills the form from a request (a preset or a history entry). Binary payloads are shown as hex.
    pub fn load(&mut self, request: &PublishRequest) {
        let properties = &request.properties;
        let (mode, payload) = match std::str::from_utf8(&request.payload) {
            Ok(text) => (if self.mode == PayloadMode::Hex { PayloadMode::Raw } else { self.mode }, text.to_owned()),
            Err(_) => (
                PayloadMode::Hex,
                request.payload.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(" "),
            ),
        };
        *self = Self {
            topic: request.topic.clone(),
            payload,
            mode,
            qos: request.qos,
            retain: request.retain,
            user_properties: properties.user_properties.clone(),
            content_type: properties.content_type.clone().unwrap_or_default(),
            response_topic: properties.response_topic.clone().unwrap_or_default(),
            correlation_data: properties
                .correlation_data
                .as_ref()
                .map_or_else(String::new, |data| String::from_utf8_lossy(data).into_owned()),
            expiry_secs: properties.message_expiry_secs.map_or_else(String::new, |seconds| seconds.to_string()),
            mark_utf8: properties.payload_is_utf8 == Some(true),
        };
    }
}

/// A light well-formedness check: every tag is closed in order and there is at least one element.
///
/// # Errors
/// Returns a description of the first problem.
pub fn check_xml(text: &str) -> Result<(), String> {
    let mut open: Vec<&str> = Vec::new();
    let mut elements = 0_usize;
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        if open.is_empty() && elements > 0 && !rest[..start].trim().is_empty() {
            return Err("invalid XML: text after the root element".to_owned());
        }
        rest = &rest[start..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.split_once("-->").ok_or("invalid XML: unterminated comment")?.1;
            continue;
        }
        if let Some(after) = rest.strip_prefix("<![CDATA[") {
            rest = after.split_once("]]>").ok_or("invalid XML: unterminated CDATA section")?.1;
            continue;
        }
        let (tag, after) = rest[1..].split_once('>').ok_or("invalid XML: unterminated tag")?;
        rest = after;
        if tag.starts_with(['?', '!']) {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            match open.pop() {
                Some(expected) if expected == name => {}
                Some(expected) => return Err(format!("invalid XML: expected </{expected}> but found </{name}>")),
                None => return Err(format!("invalid XML: unexpected </{name}>")),
            }
            continue;
        }
        let name =
            tag.split(|character: char| character.is_whitespace() || character == '/').next().unwrap_or_default();
        if name.is_empty() {
            return Err("invalid XML: tag without a name".to_owned());
        }
        if open.is_empty() && elements > 0 {
            return Err("invalid XML: more than one root element".to_owned());
        }
        elements += 1;
        if !tag.ends_with('/') {
            open.push(name);
        }
    }
    if let Some(unclosed) = open.last() {
        return Err(format!("invalid XML: <{unclosed}> is never closed"));
    }
    if elements == 0 {
        return Err("invalid XML: no element found".to_owned());
    }
    if !rest.trim().is_empty() {
        return Err("invalid XML: text after the root element".to_owned());
    }
    Ok(())
}
