//! MQTT 5: properties, reason codes and session options.

use bytes::Bytes;
use mqtt_viewer_core::config::{ConnectionProfile, LastWill, ProtocolVersion};
use mqtt_viewer_core::connection::{Command, ConnectionState, Event};
use mqtt_viewer_core::message::{Properties, PublishRequest, QoS};

use crate::common::{Client, TcpProxy, TestBroker, profile_for, refusing_broker};

#[test]
fn every_publish_property_round_trips() {
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.v5_profile());
    let properties = Properties {
        user_properties: vec![("origin".into(), "test".into()), ("origin".into(), "again".into())],
        content_type: Some("application/json".into()),
        response_topic: Some("test/reply".into()),
        correlation_data: Some(Bytes::from_static(b"\x00\x01request-7")),
        message_expiry_secs: Some(120),
        payload_is_utf8: Some(true),
    };
    let request = PublishRequest {
        topic: "test/v5".into(),
        payload: Bytes::from_static(br#"{"ok":true}"#),
        qos: QoS::AtLeastOnce,
        retain: false,
        properties: properties.clone(),
    };
    client.send(Command::Publish(request));
    let message = client.wait_message("test/v5");
    assert_eq!(&message.payload[..], br#"{"ok":true}"#);
    assert_eq!(message.properties.as_deref(), Some(&properties));
}

#[test]
fn messages_without_properties_stay_plain() {
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.v5_profile());
    for qos in QoS::ALL {
        client.send(Command::Publish(PublishRequest {
            topic: "test/plain".into(),
            payload: "x".into(),
            qos,
            ..Default::default()
        }));
    }
    let messages = client.wait_messages("test/plain", 3);
    assert!(messages.iter().all(|message| message.properties.is_none()), "{messages:?}");
}

#[test]
fn v5_and_v3_clients_interoperate() {
    let broker = TestBroker::start();
    let mut old = Client::listening(&broker.profile());
    let mut new = Client::connected(&broker.v5_profile());
    // Without properties: the test broker cannot downgrade a v5 publish that carries any.
    new.send(Command::Publish(PublishRequest {
        topic: "test/mixed".into(),
        payload: "from v5".into(),
        ..Default::default()
    }));
    let received = old.wait_message("test/mixed");
    assert_eq!(&received.payload[..], b"from v5");
    assert!(received.properties.is_none(), "3.1.1 has no properties");

    // Properties set on a 3.1.1 connection are ignored instead of breaking the publish.
    let request = PublishRequest {
        topic: "test/mixed-back".into(),
        payload: "from v3".into(),
        properties: Properties { content_type: Some("text/plain".into()), ..Default::default() },
        ..Default::default()
    };
    old.send(Command::Publish(request));
    assert_eq!(&new.wait_message("test/mixed-back").payload[..], b"from v3");
}

#[test]
fn session_options_are_accepted() {
    let broker = TestBroker::start();
    let profile = ConnectionProfile {
        session_expiry_secs: Some(300),
        topic_alias_max: Some(8),
        clean_session: false,
        client_id: "v5-session".into(),
        keep_alive_secs: 0,
        username: "someone".into(),
        password: "something".into(),
        ..broker.v5_profile()
    };
    let mut client = Client::connected(&profile);
    client.send(Command::Subscribe(mqtt_viewer_core::config::Subscription {
        filter: "extra/#".into(),
        qos: QoS::ExactlyOnce,
    }));
    client.send(Command::Publish(PublishRequest {
        topic: "extra/x".into(),
        payload: "1".into(),
        ..Default::default()
    }));
    client.wait_message("extra/x");
    client.send(Command::Unsubscribe("extra/#".into()));
    client.send(Command::Disconnect);
    client.wait_state(&ConnectionState::Disconnected);
}

#[test]
fn v5_last_will_is_published() {
    let broker = TestBroker::start();
    let proxy = TcpProxy::start(broker.v5);
    let mut watcher = Client::listening(&broker.v5_profile());
    let will = LastWill { topic: "test/v5-will".into(), payload: "gone".into(), qos: QoS::AtLeastOnce, retain: false };
    let doomed = ConnectionProfile { last_will: Some(will), protocol: ProtocolVersion::V5, ..profile_for(proxy.port) };
    let _doomed = Client::connected(&doomed);
    proxy.cut();
    assert_eq!(&watcher.wait_message("test/v5-will").payload[..], b"gone");
}

#[test]
fn v5_reason_codes_decide_between_giving_up_and_retrying() {
    // CONNACK, reason 0x86 (bad user name or password), no properties.
    let refused = refusing_broker(&[0x20, 0x03, 0x00, 0x86, 0x00]);
    let mut client = Client::new();
    client
        .send(Command::Connect(Box::new(ConnectionProfile { protocol: ProtocolVersion::V5, ..profile_for(refused) })));
    let error = client.wait_error();
    assert!(error.contains("BadUserNamePassword"), "{error}");
    client.wait_state(&ConnectionState::Disconnected);

    // Reason 0x89 (server busy) is temporary.
    let busy = refusing_broker(&[0x20, 0x03, 0x00, 0x89, 0x00]);
    let mut client = Client::new();
    client.send(Command::Connect(Box::new(ConnectionProfile { protocol: ProtocolVersion::V5, ..profile_for(busy) })));
    client.wait("a retry", |event| {
        matches!(event, Event::State(ConnectionState::Reconnecting { error, .. }) if error.contains("ServerBusy"))
            .then_some(())
    });
}
