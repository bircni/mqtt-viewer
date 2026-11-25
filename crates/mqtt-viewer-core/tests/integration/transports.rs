//! TLS and WebSocket transports, against a TLS-terminating proxy in front of the test broker.

use bytes::Bytes;
use mqtt_viewer_core::config::{ConnectionProfile, Transport};
use mqtt_viewer_core::connection::{Command, ConnectionState, Event};
use mqtt_viewer_core::message::PublishRequest;

use crate::common::{Client, TestBroker, TestPki, profile_for, tls_proxy};

fn round_trip(client: &mut Client, topic: &str) {
    let payload = Bytes::from_static(b"over the wire");
    client.send(Command::Publish(PublishRequest {
        topic: topic.into(),
        payload: payload.clone(),
        ..Default::default()
    }));
    assert_eq!(client.wait_message(topic).payload, payload, "the payload arrives unchanged");
}

fn tls_profile(port: u16) -> ConnectionProfile {
    ConnectionProfile { transport: Transport::Tls, ..profile_for(port) }
}

/// Connects and returns the error that ended the attempt, asserting that no connection was established.
fn failure(profile: ConnectionProfile) -> String {
    let mut client = Client::new();
    client.send(Command::Connect(Box::new(profile)));
    let error = client.wait("the attempt to fail", |event| match event {
        Event::Error(error) | Event::State(ConnectionState::Reconnecting { error, .. }) => Some(error.clone()),
        Event::State(_) | Event::Messages(_) | Event::ReplayProgress { .. } | Event::ReplayFinished { .. } => None,
    });
    assert!(!client.log.contains(&Event::State(ConnectionState::Connected)), "{:?}", client.log);
    error
}

#[test]
fn tls_with_a_trusted_ca_connects() {
    let (broker, pki) = (TestBroker::start(), TestPki::generate());
    let port = tls_proxy(&pki, broker.v4, false);
    let profile = ConnectionProfile { ca_file: Some(pki.file("ca.pem")), ..tls_profile(port) };
    let mut client = Client::connected(&profile);
    round_trip(&mut client, "test/tls");

    // The certificate also covers the host name.
    let by_name = ConnectionProfile { host: "localhost".into(), ..profile };
    round_trip(&mut Client::connected(&by_name), "test/tls-by-name");
}

#[test]
fn an_untrusted_certificate_is_rejected_for_good() {
    let (broker, pki) = (TestBroker::start(), TestPki::generate());
    let port = tls_proxy(&pki, broker.v4, false);
    let mut client = Client::new();
    client.send(Command::Connect(Box::new(tls_profile(port))));
    let error = client.wait_error();
    assert!(error.contains("TLS"), "{error}");
    client.wait_state(&ConnectionState::Disconnected);
    assert!(
        !client.log.iter().any(|event| matches!(event, Event::State(ConnectionState::Reconnecting { .. }))),
        "a certificate problem does not fix itself, so there is no point in retrying"
    );
}

#[test]
fn a_certificate_for_another_host_is_rejected() {
    // The certificate names only `localhost`, the client connects by address.
    let (broker, pki) = (TestBroker::start(), TestPki::generate_for(&["localhost"]));
    let port = tls_proxy(&pki, broker.v4, false);
    let profile = ConnectionProfile { ca_file: Some(pki.file("ca.pem")), ..tls_profile(port) };
    let error = failure(profile.clone());
    assert!(error.contains("certificate"), "{error}");
    round_trip(&mut Client::connected(&ConnectionProfile { host: "localhost".into(), ..profile }), "test/right-name");
}

#[test]
fn validation_can_be_switched_off() {
    let (broker, pki) = (TestBroker::start(), TestPki::generate());
    let port = tls_proxy(&pki, broker.v4, false);
    let profile = ConnectionProfile { validate_certificate: false, ..tls_profile(port) };
    round_trip(&mut Client::connected(&profile), "test/insecure");
}

#[test]
fn mutual_tls_needs_the_client_certificate() {
    let (broker, pki) = (TestBroker::start(), TestPki::generate());
    let port = tls_proxy(&pki, broker.v4, true);
    let trusted = ConnectionProfile { ca_file: Some(pki.file("ca.pem")), ..tls_profile(port) };
    let with_certificate = ConnectionProfile {
        client_cert_file: Some(pki.file("client.pem")),
        client_key_file: Some(pki.file("client.key")),
        ..trusted.clone()
    };
    round_trip(&mut Client::connected(&with_certificate), "test/mtls");

    let error = failure(trusted);
    assert!(!error.is_empty(), "without a certificate the server turns the client away");
}

#[test]
fn unusable_certificate_files_are_reported_by_name() {
    let pki = TestPki::generate();
    let garbage = pki.file("garbage.pem");
    std::fs::write(&garbage, "not a certificate").unwrap();
    let base = tls_profile(8883);

    let error = failure(ConnectionProfile { ca_file: Some(pki.file("missing.pem")), ..base.clone() });
    assert!(error.contains("missing.pem"), "{error}");
    let error = failure(ConnectionProfile { ca_file: Some(garbage.clone()), ..base.clone() });
    assert!(error.contains("contains no certificates"), "{error}");

    let error = failure(ConnectionProfile {
        client_cert_file: Some(pki.file("client.pem")),
        client_key_file: Some(garbage.clone()),
        ..base.clone()
    });
    assert!(error.contains("client key"), "{error}");
    let error = failure(ConnectionProfile {
        client_cert_file: Some(garbage),
        client_key_file: Some(pki.file("client.key")),
        ..base.clone()
    });
    assert!(error.contains("garbage.pem"), "{error}");
    // A key that does not belong to the certificate.
    let other = TestPki::generate();
    let error = failure(ConnectionProfile {
        client_cert_file: Some(pki.file("client.pem")),
        client_key_file: Some(other.file("client.key")),
        ..base
    });
    assert!(error.contains("client certificate"), "{error}");
}

#[test]
fn websocket_transport_connects() {
    let broker = TestBroker::start();
    round_trip(&mut Client::connected(&broker.ws_profile()), "test/ws");
    // A path without the leading slash is tolerated.
    let sloppy = ConnectionProfile { ws_path: "mqtt".into(), ..broker.ws_profile() };
    round_trip(&mut Client::connected(&sloppy), "test/ws-path");
}

#[test]
fn secure_websocket_transport_connects() {
    let (broker, pki) = (TestBroker::start(), TestPki::generate());
    let port = tls_proxy(&pki, broker.ws, false);
    let profile =
        ConnectionProfile { transport: Transport::Wss, port, ca_file: Some(pki.file("ca.pem")), ..broker.ws_profile() };
    round_trip(&mut Client::connected(&profile), "test/wss");
}
