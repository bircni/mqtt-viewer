use bytes::Bytes;

use crate::message::{Message, Properties, PublishRequest, QoS};

#[test]
fn qos_round_trips_through_its_level() {
    for qos in QoS::ALL {
        assert_eq!(QoS::try_from(qos.level()), Ok(qos));
        assert_eq!(u8::from(qos), qos.level());
    }
    QoS::try_from(3).unwrap_err();
}

#[test]
fn qos_serializes_as_number() {
    assert_eq!(serde_json::to_string(&QoS::ExactlyOnce).unwrap(), "2");
    assert_eq!(serde_json::from_str::<QoS>("1").unwrap(), QoS::AtLeastOnce);
    serde_json::from_str::<QoS>("7").unwrap_err();
}

#[test]
fn properties_know_when_they_are_empty() {
    assert!(Properties::default().is_empty());
    for properties in [
        Properties { user_properties: vec![("k".into(), "v".into())], ..Properties::default() },
        Properties { content_type: Some("text/plain".into()), ..Properties::default() },
        Properties { response_topic: Some("r".into()), ..Properties::default() },
        Properties { correlation_data: Some(Bytes::from_static(b"\x00\x01")), ..Properties::default() },
        Properties { message_expiry_secs: Some(5), ..Properties::default() },
        Properties { payload_is_utf8: Some(true), ..Properties::default() },
    ] {
        assert!(!properties.is_empty(), "{properties:?}");
    }
}

#[test]
fn properties_round_trip_through_json() {
    let properties = Properties {
        user_properties: vec![("a".into(), "1".into()), ("a".into(), "2".into())],
        content_type: Some("application/json".into()),
        response_topic: Some("reply/here".into()),
        correlation_data: Some(Bytes::from_static(b"\x00\xff\x10")),
        message_expiry_secs: Some(30),
        payload_is_utf8: Some(false),
    };
    let json = serde_json::to_string(&properties).unwrap();
    assert!(json.contains("\"correlation_data\":\"AP8Q\""), "{json}");
    assert_eq!(serde_json::from_str::<Properties>(&json).unwrap(), properties);
    assert_eq!(serde_json::to_string(&Properties::default()).unwrap(), "{}");
    assert_eq!(serde_json::from_str::<Properties>("{\"correlation_data\":null}").unwrap(), Properties::default());
    serde_json::from_str::<Properties>("{\"correlation_data\":\"***\"}").unwrap_err();
}

#[test]
fn publish_request_round_trips_binary_payloads() {
    let request = PublishRequest {
        topic: "a/b".into(),
        payload: Bytes::from_static(b"\x00\x01\xfe"),
        qos: QoS::AtLeastOnce,
        retain: true,
        properties: Properties::default(),
    };
    let text = toml::to_string(&request).unwrap();
    assert!(!text.contains("properties"), "{text}");
    assert_eq!(toml::from_str::<PublishRequest>(&text).unwrap(), request);
    toml::from_str::<PublishRequest>("topic = \"a\"\npayload = \"not base64!\"").unwrap_err();
}

#[test]
fn delete_request_is_an_empty_retained_message() {
    let request = PublishRequest::delete_retained("a/b");
    assert_eq!(request.topic, "a/b");
    assert!(request.payload.is_empty());
    assert!(request.retain);
}

#[test]
fn new_message_has_plain_defaults() {
    let message = Message::new("a/b", "hi");
    assert_eq!(message.topic, "a/b");
    assert_eq!(&message.payload[..], b"hi");
    assert_eq!(message.qos, QoS::AtMostOnce);
    assert!(!message.retain);
    assert!(message.properties.is_none());
}
