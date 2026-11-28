//! One MQTT session, independent of the protocol version.
//!
//! `rumqttc` has separate client and event types for MQTT 3.1.1 and 5. [`Client`] and [`EventLoop`] wrap both
//! so the worker never branches on the version.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use jiff::Timestamp;
use rumqttc::v5::mqttbytes::v5 as p5;
use rumqttc::{TlsConfiguration, Transport as WireTransport};

use super::tls;
use crate::config::{ConnectionProfile, ProtocolVersion, Transport};
use crate::message::{Message, Properties, PublishRequest, QoS};

/// Largest packet MQTT allows; the library default of 10 KiB would reject ordinary payloads.
const MAX_PACKET_SIZE: u32 = 268_435_455;
/// Requests buffered between the worker and the network task.
const REQUEST_CAPACITY: usize = 4096;

/// What a poll of the event loop produced.
pub(super) enum Incoming {
    /// The broker accepted the connection.
    Connected,
    /// A message arrived.
    Publish(Message),
    /// The broker reported a problem that does not end the session.
    Warning(String),
    /// Our own disconnect went out; the session is over.
    Closed,
    /// Anything the worker does not care about.
    Other,
}

/// A failed poll.
pub(super) struct SessionError {
    /// Whether retrying is pointless (bad credentials, bad certificate, ...).
    pub fatal: bool,
    /// Description for the user.
    pub message: String,
}

/// The sending half of a session.
#[derive(Clone)]
pub(super) enum Client {
    V4(rumqttc::AsyncClient),
    V5(rumqttc::v5::AsyncClient),
}

/// The receiving half of a session; polling it drives the network.
pub(super) enum EventLoop {
    V4(Box<rumqttc::EventLoop>),
    V5(Box<rumqttc::v5::EventLoop>),
}

/// Creates the two halves of a session for `profile`. Nothing touches the network until the first poll.
pub(super) fn open(profile: &ConnectionProfile) -> Result<(Client, EventLoop), String> {
    profile.validate()?;
    let client_id = profile.effective_client_id();
    let (address, transport) = wire_transport(profile)?;
    // The library panics on keep-alive values below 5 seconds, and its MQTT 5 client on 0 ("off") as well;
    // there the longest interval the protocol allows is the closest thing to "off".
    let keep_alive = match (profile.keep_alive_secs, profile.protocol) {
        (0, ProtocolVersion::V311) => Duration::ZERO,
        (0, ProtocolVersion::V5) => Duration::from_secs(u64::from(u16::MAX)),
        (seconds, _) => Duration::from_secs(u64::from(seconds.max(5))),
    };
    let credentials = credentials(profile);

    match profile.protocol {
        ProtocolVersion::V311 => {
            let mut options = rumqttc::MqttOptions::new(client_id, address, profile.port);
            options.set_transport(transport).set_keep_alive(keep_alive).set_clean_session(profile.clean_session);
            options.set_max_packet_size(MAX_PACKET_SIZE as usize, MAX_PACKET_SIZE as usize);
            if let Some((username, password)) = credentials {
                options.set_credentials(username, password);
            }
            if let Some(will) = profile.last_will.as_ref().filter(|will| !will.topic.is_empty()) {
                options.set_last_will(rumqttc::LastWill::new(
                    will.topic.clone(),
                    will.payload.clone(),
                    qos_v4(will.qos),
                    will.retain,
                ));
            }
            let (client, eventloop) = rumqttc::AsyncClient::new(options, REQUEST_CAPACITY);
            Ok((Client::V4(client), EventLoop::V4(Box::new(eventloop))))
        }
        ProtocolVersion::V5 => {
            let mut options = rumqttc::v5::MqttOptions::new(client_id, address, profile.port);
            options.set_transport(transport).set_keep_alive(keep_alive).set_clean_start(profile.clean_session);
            options.set_max_packet_size(Some(MAX_PACKET_SIZE));
            options.set_session_expiry_interval(profile.session_expiry_secs);
            options.set_topic_alias_max(profile.topic_alias_max);
            if let Some((username, password)) = credentials {
                options.set_credentials(username, password);
            }
            if let Some(will) = profile.last_will.as_ref().filter(|will| !will.topic.is_empty()) {
                options.set_last_will(p5::LastWill::new(
                    will.topic.clone(),
                    will.payload.clone(),
                    qos_v5(will.qos),
                    will.retain,
                    None,
                ));
            }
            let (client, eventloop) = rumqttc::v5::AsyncClient::new(options, REQUEST_CAPACITY);
            Ok((Client::V5(client), EventLoop::V5(Box::new(eventloop))))
        }
    }
}

/// Username and password to send, if any. MQTT 5 allows a password without a username (token logins);
/// [`ConnectionProfile::validate`] rejects that combination for MQTT 3.1.1.
fn credentials(profile: &ConnectionProfile) -> Option<(String, String)> {
    (!profile.username.is_empty() || !profile.password.is_empty())
        .then(|| (profile.username.clone(), profile.password.clone()))
}

fn wire_transport(profile: &ConnectionProfile) -> Result<(String, WireTransport), String> {
    let tls = || tls::client_config(profile).map(|config| TlsConfiguration::Rustls(Arc::new(config)));
    let path = if profile.ws_path.starts_with('/') { profile.ws_path.clone() } else { format!("/{}", profile.ws_path) };
    let host = profile.host.trim();
    Ok(match profile.transport {
        Transport::Tcp => (host.to_owned(), WireTransport::Tcp),
        Transport::Tls => (host.to_owned(), WireTransport::tls_with_config(tls()?)),
        // For WebSocket the library expects the full URL in place of the host.
        Transport::Ws => (format!("ws://{host}:{}{path}", profile.port), WireTransport::Ws),
        Transport::Wss => (format!("wss://{host}:{}{path}", profile.port), WireTransport::wss_with_config(tls()?)),
    })
}

const fn qos_v4(qos: QoS) -> rumqttc::QoS {
    match qos {
        QoS::AtMostOnce => rumqttc::QoS::AtMostOnce,
        QoS::AtLeastOnce => rumqttc::QoS::AtLeastOnce,
        QoS::ExactlyOnce => rumqttc::QoS::ExactlyOnce,
    }
}

const fn qos_v5(qos: QoS) -> rumqttc::v5::mqttbytes::QoS {
    match qos {
        QoS::AtMostOnce => rumqttc::v5::mqttbytes::QoS::AtMostOnce,
        QoS::AtLeastOnce => rumqttc::v5::mqttbytes::QoS::AtLeastOnce,
        QoS::ExactlyOnce => rumqttc::v5::mqttbytes::QoS::ExactlyOnce,
    }
}

const fn from_v4(qos: rumqttc::QoS) -> QoS {
    match qos {
        rumqttc::QoS::AtMostOnce => QoS::AtMostOnce,
        rumqttc::QoS::AtLeastOnce => QoS::AtLeastOnce,
        rumqttc::QoS::ExactlyOnce => QoS::ExactlyOnce,
    }
}

const fn from_v5(qos: rumqttc::v5::mqttbytes::QoS) -> QoS {
    match qos {
        rumqttc::v5::mqttbytes::QoS::AtMostOnce => QoS::AtMostOnce,
        rumqttc::v5::mqttbytes::QoS::AtLeastOnce => QoS::AtLeastOnce,
        rumqttc::v5::mqttbytes::QoS::ExactlyOnce => QoS::ExactlyOnce,
    }
}

/// The request could not be queued because the network task is behind; try again later.
pub(super) struct Busy;

impl Client {
    pub(super) fn subscribe(&self, filter: &str, qos: QoS) -> Result<(), Busy> {
        match self {
            Self::V4(client) => client.try_subscribe(filter, qos_v4(qos)).map_err(|_| Busy),
            Self::V5(client) => client.try_subscribe(filter, qos_v5(qos)).map_err(|_| Busy),
        }
    }

    pub(super) fn unsubscribe(&self, filter: &str) -> Result<(), Busy> {
        match self {
            Self::V4(client) => client.try_unsubscribe(filter).map_err(|_| Busy),
            Self::V5(client) => client.try_unsubscribe(filter).map_err(|_| Busy),
        }
    }

    pub(super) fn publish(&self, request: &PublishRequest) -> Result<(), Busy> {
        match self {
            Self::V4(client) => client
                .try_publish(request.topic.as_str(), qos_v4(request.qos), request.retain, request.payload.clone())
                .map_err(|_| Busy),
            Self::V5(client) => {
                let topic = request.topic.as_str();
                let qos = qos_v5(request.qos);
                let payload = request.payload.clone();
                if request.properties.is_empty() {
                    client.try_publish(topic, qos, request.retain, payload).map_err(|_| Busy)
                } else {
                    let properties = to_wire_properties(&request.properties);
                    client
                        .try_publish_with_properties(topic, qos, request.retain, payload, properties)
                        .map_err(|_| Busy)
                }
            }
        }
    }

    pub(super) fn disconnect(&self) {
        // A full queue means the session is being torn down anyway.
        let _ = match self {
            Self::V4(client) => client.try_disconnect().map_err(|_| Busy),
            Self::V5(client) => client.try_disconnect().map_err(|_| Busy),
        };
    }
}

fn to_wire_properties(properties: &Properties) -> p5::PublishProperties {
    p5::PublishProperties {
        payload_format_indicator: properties.payload_is_utf8.map(u8::from),
        message_expiry_interval: properties.message_expiry_secs,
        topic_alias: None,
        response_topic: properties.response_topic.clone(),
        correlation_data: properties.correlation_data.clone(),
        user_properties: properties.user_properties.clone(),
        subscription_identifiers: Vec::new(),
        content_type: properties.content_type.clone(),
    }
}

fn from_wire_properties(properties: p5::PublishProperties) -> Option<Box<Properties>> {
    let properties = Properties {
        user_properties: properties.user_properties,
        content_type: properties.content_type,
        response_topic: properties.response_topic,
        correlation_data: properties.correlation_data.map(|data| Bytes::copy_from_slice(&data)),
        message_expiry_secs: properties.message_expiry_interval,
        payload_is_utf8: properties.payload_format_indicator.map(|indicator| indicator == 1),
    };
    (!properties.is_empty()).then(|| Box::new(properties))
}

impl EventLoop {
    /// Waits for the next network event, connecting first when necessary.
    pub(super) async fn poll(&mut self) -> Result<Incoming, SessionError> {
        match self {
            Self::V4(eventloop) => eventloop.poll().await.map(incoming_v4).map_err(|error| error_v4(&error)),
            Self::V5(eventloop) => eventloop.poll().await.map(incoming_v5).map_err(|error| error_v5(&error)),
        }
    }
}

fn incoming_v4(event: rumqttc::Event) -> Incoming {
    match event {
        rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_)) => Incoming::Connected,
        rumqttc::Event::Incoming(rumqttc::Packet::Publish(publish)) => Incoming::Publish(Message {
            topic: publish.topic,
            // Copy: the payload is a slice of the read buffer and would keep the whole buffer alive.
            payload: Bytes::copy_from_slice(&publish.payload),
            qos: from_v4(publish.qos),
            retain: publish.retain,
            received_at: Timestamp::now(),
            properties: None,
        }),
        rumqttc::Event::Incoming(rumqttc::Packet::SubAck(ack))
            if ack.return_codes.contains(&rumqttc::SubscribeReasonCode::Failure) =>
        {
            Incoming::Warning("the broker rejected a subscription".to_owned())
        }
        rumqttc::Event::Outgoing(rumqttc::Outgoing::Disconnect) => Incoming::Closed,
        rumqttc::Event::Incoming(_) | rumqttc::Event::Outgoing(_) => Incoming::Other,
    }
}

fn incoming_v5(event: rumqttc::v5::Event) -> Incoming {
    match event {
        rumqttc::v5::Event::Incoming(p5::Packet::ConnAck(_)) => Incoming::Connected,
        rumqttc::v5::Event::Incoming(p5::Packet::Publish(publish)) => Incoming::Publish(Message {
            topic: String::from_utf8_lossy(&publish.topic).into_owned(),
            payload: Bytes::copy_from_slice(&publish.payload),
            qos: from_v5(publish.qos),
            retain: publish.retain,
            received_at: Timestamp::now(),
            properties: publish.properties.and_then(from_wire_properties),
        }),
        rumqttc::v5::Event::Incoming(p5::Packet::SubAck(ack)) => {
            let rejected: Vec<String> = ack
                .return_codes
                .iter()
                .filter(|code| !matches!(code, p5::SubscribeReasonCode::Success(_)))
                .map(|code| format!("{code:?}"))
                .collect();
            if rejected.is_empty() {
                Incoming::Other
            } else {
                Incoming::Warning(format!("the broker rejected a subscription: {}", rejected.join(", ")))
            }
        }
        rumqttc::v5::Event::Incoming(p5::Packet::Disconnect(disconnect)) => {
            let reason = disconnect.properties.and_then(|properties| properties.reason_string).unwrap_or_default();
            Incoming::Warning(
                format!("the broker disconnected: {:?} {reason}", disconnect.reason_code).trim().to_owned(),
            )
        }
        rumqttc::v5::Event::Outgoing(rumqttc::Outgoing::Disconnect) => Incoming::Closed,
        rumqttc::v5::Event::Incoming(_) | rumqttc::v5::Event::Outgoing(_) => Incoming::Other,
    }
}

fn error_v4(error: &rumqttc::ConnectionError) -> SessionError {
    use rumqttc::ConnectReturnCode as Code;
    use rumqttc::ConnectionError as Error;
    let refused = matches!(
        error,
        Error::ConnectionRefused(
            Code::BadUserNamePassword | Code::NotAuthorized | Code::BadClientId | Code::RefusedProtocolVersion
        )
    );
    let bad_setup = matches!(error, Error::Tls(tls_error) if !matches!(tls_error, rumqttc::TlsError::Io(_)));
    SessionError { fatal: refused || bad_setup || certificate_rejected(error), message: describe(error) }
}

fn error_v5(error: &rumqttc::v5::ConnectionError) -> SessionError {
    use p5::ConnectReturnCode as Code;
    use rumqttc::v5::ConnectionError as Error;
    let refused = matches!(
        error,
        Error::ConnectionRefused(
            Code::BadUserNamePassword
                | Code::NotAuthorized
                | Code::Banned
                | Code::BadAuthenticationMethod
                | Code::ClientIdentifierNotValid
                | Code::BadClientId
                | Code::UnsupportedProtocolVersion
                | Code::RefusedProtocolVersion
        )
    );
    let bad_setup = matches!(error, Error::Tls(tls_error) if !matches!(tls_error, rumqttc::TlsError::Io(_)));
    SessionError { fatal: refused || bad_setup || certificate_rejected(error), message: describe(error) }
}

/// Whether the TLS handshake failed because the server's certificate was not acceptable. Retrying cannot fix
/// that, unlike a network hiccup during the handshake. The TLS library reports both wrapped in I/O errors, so
/// the whole chain of causes is searched.
fn certificate_rejected(error: &(dyn std::error::Error + 'static)) -> bool {
    use rumqttc::tokio_rustls::rustls::Error as TlsError;
    let mut current = Some(error);
    while let Some(cause) = current {
        if matches!(cause.downcast_ref::<TlsError>(), Some(TlsError::InvalidCertificate(_))) {
            return true;
        }
        let wrapped = cause.downcast_ref::<std::io::Error>().and_then(std::io::Error::get_ref);
        if let Some(inner) = wrapped
            && certificate_rejected(inner)
        {
            return true;
        }
        current = cause.source();
    }
    false
}

/// Renders an error with its chain of causes; the top-level text alone is often just "I/O".
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}
