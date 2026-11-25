//! The message types shared by the connection, the tree and the recorder.

use bytes::Bytes;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// MQTT quality of service level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(into = "u8", try_from = "u8")]
pub enum QoS {
    /// QoS 0: fire and forget.
    #[default]
    AtMostOnce,
    /// QoS 1: acknowledged delivery.
    AtLeastOnce,
    /// QoS 2: assured delivery.
    ExactlyOnce,
}

impl QoS {
    /// All levels, in ascending order.
    pub const ALL: [Self; 3] = [Self::AtMostOnce, Self::AtLeastOnce, Self::ExactlyOnce];

    /// The numeric level (0, 1 or 2).
    #[must_use]
    pub const fn level(self) -> u8 {
        match self {
            Self::AtMostOnce => 0,
            Self::AtLeastOnce => 1,
            Self::ExactlyOnce => 2,
        }
    }
}

impl From<QoS> for u8 {
    fn from(qos: QoS) -> Self {
        qos.level()
    }
}

impl TryFrom<u8> for QoS {
    type Error = String;

    fn try_from(level: u8) -> Result<Self, Self::Error> {
        match level {
            0 => Ok(Self::AtMostOnce),
            1 => Ok(Self::AtLeastOnce),
            2 => Ok(Self::ExactlyOnce),
            other => Err(format!("invalid QoS level {other}")),
        }
    }
}

/// MQTT 5 publish properties. All fields are empty for MQTT 3.1.1 messages.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Properties {
    /// Free-form key/value pairs; keys may repeat.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub user_properties: Vec<(String, String)>,
    /// MIME type of the payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Topic a responder should answer on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_topic: Option<String>,
    /// Opaque data to correlate a response with its request.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt_bytes_b64")]
    pub correlation_data: Option<Bytes>,
    /// Seconds after which the broker discards the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_expiry_secs: Option<u32>,
    /// `Some(true)` when the sender declared the payload to be UTF-8 text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_is_utf8: Option<bool>,
}

impl Properties {
    /// Whether no property is set.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.user_properties.is_empty()
            && self.content_type.is_none()
            && self.response_topic.is_none()
            && self.correlation_data.is_none()
            && self.message_expiry_secs.is_none()
            && self.payload_is_utf8.is_none()
    }
}

/// A message received from the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Full topic name.
    pub topic: String,
    /// Raw payload.
    pub payload: Bytes,
    /// QoS the message was delivered with.
    pub qos: QoS,
    /// Whether the broker flagged the message as retained.
    pub retain: bool,
    /// When this client received it.
    pub received_at: Timestamp,
    /// MQTT 5 properties, if any were set.
    pub properties: Option<Box<Properties>>,
}

impl Message {
    /// A QoS 0, non-retained message received now. Mainly for tests and tools.
    #[must_use]
    pub fn new(topic: impl Into<String>, payload: impl Into<Bytes>) -> Self {
        Self {
            topic: topic.into(),
            payload: payload.into(),
            qos: QoS::AtMostOnce,
            retain: false,
            received_at: Timestamp::now(),
            properties: None,
        }
    }
}

/// A message to send to the broker.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PublishRequest {
    /// Topic to publish to.
    pub topic: String,
    /// Payload bytes.
    #[serde(with = "bytes_b64")]
    pub payload: Bytes,
    /// Requested QoS.
    #[serde(default)]
    pub qos: QoS,
    /// Whether the broker should retain the message.
    #[serde(default)]
    pub retain: bool,
    /// MQTT 5 properties; ignored on 3.1.1 connections.
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub properties: Properties,
}

impl PublishRequest {
    /// The request that makes a broker drop the retained message on `topic`.
    #[must_use]
    pub fn delete_retained(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            payload: Bytes::new(),
            qos: QoS::AtMostOnce,
            retain: true,
            properties: Properties::default(),
        }
    }
}

/// Serde adapter storing bytes as Base64 text.
pub mod bytes_b64 {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize as Base64.
    ///
    /// # Errors
    /// Propagates serializer errors.
    pub fn serialize<S: Serializer>(bytes: &Bytes, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    /// Deserialize from Base64.
    ///
    /// # Errors
    /// Fails when the text is not valid Base64.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Bytes, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text).map(Bytes::from).map_err(serde::de::Error::custom)
    }
}

mod opt_bytes_b64 {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serializer};

    #[expect(clippy::ref_option, reason = "signature dictated by serde's `with`")]
    pub fn serialize<S: Serializer>(bytes: &Option<Bytes>, serializer: S) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(bytes) => serializer.serialize_some(&STANDARD.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Bytes>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        text.map(|text| STANDARD.decode(text).map(Bytes::from).map_err(serde::de::Error::custom)).transpose()
    }
}
