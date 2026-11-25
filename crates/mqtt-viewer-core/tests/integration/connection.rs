//! End-to-end tests of the connection worker against an in-process broker (MQTT 3.1.1 over TCP).

use std::time::Duration;

use crate::common::{BrokerOptions, Client, TcpProxy, TestBroker, free_port, profile_for, refusing_broker};
use bytes::Bytes;
use mqtt_viewer_core::config::{ConnectionProfile, LastWill, Subscription};
use mqtt_viewer_core::connection::{Command, ConnectionState, Event};
use mqtt_viewer_core::message::{PublishRequest, QoS};
use mqtt_viewer_core::record::ReplayStep;

fn publish(topic: &str, payload: &'static str) -> Command {
    Command::Publish(PublishRequest {
        topic: topic.into(),
        payload: Bytes::from_static(payload.as_bytes()),
        ..Default::default()
    })
}

#[test]
fn connect_subscribe_publish_receive() {
    let broker = TestBroker::start();
    let mut client = Client::new();
    assert!(client.connection.try_event().is_none());
    client.send(Command::Connect(Box::new(broker.profile())));
    client.wait_state(&ConnectionState::Connecting);
    client.wait_state(&ConnectionState::Connected);

    for qos in QoS::ALL {
        let request = PublishRequest {
            topic: "test/qos".into(),
            payload: Bytes::from(vec![qos.level()]),
            qos,
            ..Default::default()
        };
        client.send(Command::Publish(request));
    }
    let received = client.wait_messages("test/qos", 3);
    let payloads: Vec<u8> = received.iter().map(|message| message.payload[0]).collect();
    assert_eq!(payloads, [0, 1, 2], "messages arrive in publish order");
    assert!(received.iter().all(|message| !message.retain && message.properties.is_none()));

    client.send(publish("other/topic", "not subscribed"));
    client.send(publish("test/after", "marker"));
    client.wait_message("test/after");
    assert!(
        !client
            .log
            .iter()
            .any(|event| matches!(event, Event::Messages(batch) if batch.iter().any(|m| m.topic == "other/topic")))
    );

    client.send(Command::Disconnect);
    client.wait_state(&ConnectionState::Disconnected);
    assert!(ConnectionState::Connected.is_active());
    assert!(!ConnectionState::Disconnected.is_active());
}

#[test]
fn large_payloads_pass_through() {
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.profile());
    let payload = Bytes::from(vec![0xAB_u8; 1024 * 1024]);
    client.send(Command::Publish(PublishRequest {
        topic: "test/big".into(),
        payload: payload.clone(),
        ..Default::default()
    }));
    assert_eq!(client.wait_message("test/big").payload, payload);
}

#[test]
fn runtime_subscriptions_can_be_added_and_removed() {
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.profile());
    client.send(Command::Subscribe(Subscription { filter: "extra/+".into(), qos: QoS::AtLeastOnce }));
    // Subscribing twice must not deliver twice.
    client.send(Command::Subscribe(Subscription::new("extra/+")));
    client.send(publish("extra/a", "1"));
    client.wait_message("extra/a");

    client.send(Command::Unsubscribe("extra/+".into()));
    client.send(publish("extra/a", "2"));
    client.send(publish("test/marker", "done"));
    client.wait_message("test/marker");
    let extra: Vec<_> = client
        .log
        .iter()
        .filter_map(|event| if let Event::Messages(batch) = event { Some(batch) } else { None })
        .flatten()
        .filter(|message| message.topic == "extra/a")
        .collect();
    assert_eq!(extra.len(), 1, "{extra:?}");
}

#[test]
fn two_clients_see_each_others_messages() {
    let broker = TestBroker::start();
    let mut alice = Client::connected(&ConnectionProfile { client_id: "alice".into(), ..broker.profile() });
    let mut bob = Client::connected(&broker.profile());
    alice.send(publish("test/chat", "hi bob"));
    assert_eq!(&bob.wait_message("test/chat").payload[..], b"hi bob");
    assert_eq!(&alice.wait_message("test/chat").payload[..], b"hi bob");
}

#[test]
fn refused_credentials_end_in_disconnected_without_retrying() {
    // CONNACK with return code 4: bad user name or password.
    let port = refusing_broker(&[0x20, 0x02, 0x00, 0x04]);
    let mut client = Client::new();
    client.send(Command::Connect(Box::new(profile_for(port))));
    let error = client.wait_error();
    assert!(error.contains("BadUserNamePassword"), "{error}");
    client.wait_state(&ConnectionState::Disconnected);
    assert!(!client.log.iter().any(|event| matches!(event, Event::State(ConnectionState::Reconnecting { .. }))));
    // The worker is idle again and accepts a new attempt.
    client.send(publish("a", "b"));
    assert_eq!(client.wait_error(), "cannot publish: not connected");
}

#[test]
fn temporary_refusals_are_retried() {
    // CONNACK with return code 3: server unavailable.
    let port = refusing_broker(&[0x20, 0x02, 0x00, 0x03]);
    let mut client = Client::new();
    client.send(Command::Connect(Box::new(profile_for(port))));
    client.wait("a retry", |event| {
        matches!(event, Event::State(ConnectionState::Reconnecting { error, .. }) if error.contains("ServiceUnavailable")).then_some(())
    });
}

#[test]
fn broker_side_credentials_are_sent() {
    let broker = TestBroker::start_with(&BrokerOptions { credentials: Some(("user", "secret")) });
    let mut client = Client::new();
    let wrong = ConnectionProfile { username: "user".into(), password: "nope".into(), ..broker.profile() };
    client.send(Command::Connect(Box::new(wrong)));
    // This broker hangs up on bad credentials instead of refusing, which looks like an outage.
    client.wait("a retry", |event| matches!(event, Event::State(ConnectionState::Reconnecting { .. })).then_some(()));

    let right = ConnectionProfile { username: "user".into(), password: "secret".into(), ..broker.profile() };
    client.send(Command::Connect(Box::new(right)));
    client.wait_state(&ConnectionState::Connected);
}

#[test]
fn unreachable_broker_is_retried_until_it_appears() {
    let port = free_port();
    let mut client = Client::new();
    client.send(Command::Connect(Box::new(profile_for(port))));
    let first = client.wait("a retry", |event| {
        if let Event::State(ConnectionState::Reconnecting { attempt, error }) = event {
            Some((*attempt, error.clone()))
        } else {
            None
        }
    });
    assert_eq!(first.0, 1);
    assert_ne!(first.1.len(), 0);
    client.wait("a second retry", |event| {
        matches!(event, Event::State(ConnectionState::Reconnecting { attempt: 2, .. })).then_some(())
    });

    client.send(Command::Disconnect);
    client.wait_state(&ConnectionState::Disconnected);
    assert_eq!(client.messages_within(Duration::from_millis(1200)).len(), 0);
    assert!(
        !client
            .log
            .iter()
            .rev()
            .take_while(|event| **event != Event::State(ConnectionState::Disconnected))
            .any(|event| matches!(event, Event::State(_))),
        "no retries after an explicit disconnect"
    );
}

#[test]
fn connection_recovers_when_the_broker_comes_back() {
    let broker = TestBroker::start();
    let proxy = TcpProxy::start(broker.v4);
    let mut client = Client::connected(&profile_for(proxy.port));
    client.send(publish("test/before", "1"));
    client.wait_message("test/before");

    proxy.cut();
    client.wait("the outage to be noticed", |event| {
        matches!(event, Event::State(ConnectionState::Reconnecting { .. })).then_some(())
    });
    // Publishes made while offline are queued, not lost.
    client.send(publish("test/during", "queued"));

    proxy.restore();
    client.wait_state(&ConnectionState::Connected);
    assert_eq!(&client.wait_message("test/during").payload[..], b"queued");
    // The subscription was re-established, so new traffic flows again.
    client.send(publish("test/after", "2"));
    client.wait_message("test/after");
}

#[test]
fn reconnecting_with_a_new_profile_replaces_the_old_connection() {
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.profile());
    let narrower = ConnectionProfile { subscriptions: vec![Subscription::new("only/this")], ..broker.profile() };
    client.send(Command::Connect(Box::new(narrower)));
    client.wait_state(&ConnectionState::Connecting);
    client.wait_state(&ConnectionState::Connected);
    client.send(publish("test/old", "ignored"));
    client.send(publish("only/this", "seen"));
    client.wait_message("only/this");
    assert!(
        !client
            .log
            .iter()
            .any(|event| matches!(event, Event::Messages(batch) if batch.iter().any(|m| m.topic == "test/old")))
    );
}

#[test]
fn commands_without_a_connection_are_reported() {
    let mut client = Client::new();
    client.send(publish("a", "b"));
    assert_eq!(client.wait_error(), "cannot publish: not connected");
    client.send(Command::Subscribe(Subscription::new("a")));
    assert_eq!(client.wait_error(), "cannot subscribe: not connected");
    client.send(Command::Unsubscribe("a".into()));
    assert_eq!(client.wait_error(), "cannot unsubscribe: not connected");
    client.send(Command::Replay(vec![ReplayStep { delay: Duration::ZERO, request: PublishRequest::default() }]));
    assert_eq!(client.wait_error(), "cannot replay: not connected");
    client.wait("replay end", |event| (*event == Event::ReplayFinished { sent: 0, total: 1 }).then_some(()));
    // Disconnecting and stopping a replay when idle is a quiet no-op.
    client.send(Command::Disconnect);
    client.send(Command::StopReplay);
    assert!(client.connection.wait_event(Duration::from_millis(200)).is_none());
}

#[test]
fn invalid_requests_are_rejected_before_they_reach_the_broker() {
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.profile());
    client.send(publish("test/+/wild", "x"));
    assert!(client.wait_error().contains("wildcards"));
    client.send(publish("", "x"));
    assert!(client.wait_error().contains("empty"));

    let mut idle = Client::new();
    idle.send(Command::Connect(Box::new(ConnectionProfile { host: String::new(), ..broker.profile() })));
    assert!(idle.wait_error().contains("host"));
    idle.wait_state(&ConnectionState::Disconnected);
}

#[test]
fn retained_messages_are_delivered_to_late_subscribers_and_can_be_deleted() {
    let broker = TestBroker::start();
    let mut writer = Client::connected(&broker.profile());
    let retained = PublishRequest {
        topic: "test/retained".into(),
        payload: "kept".into(),
        qos: QoS::AtLeastOnce,
        retain: true,
        ..Default::default()
    };
    writer.send(Command::Publish(retained));
    writer.wait_message("test/retained");

    let mut late = Client::connected(&broker.profile());
    let message = late.wait_message("test/retained");
    assert_eq!(&message.payload[..], b"kept");
    assert!(message.retain, "a message delivered on subscribe carries the retain flag");

    writer.send(Command::Publish(PublishRequest::delete_retained("test/retained")));
    writer.send(publish("test/marker", "sync"));
    writer.wait_message("test/marker");
    let mut later = Client::connected(&broker.profile());
    later.send(publish("test/marker2", "sync"));
    later.wait_message("test/marker2");
    let leftovers: Vec<_> = later
        .log
        .iter()
        .filter_map(|event| if let Event::Messages(batch) = event { Some(batch) } else { None })
        .flatten()
        .filter(|message| message.topic == "test/retained" && !message.payload.is_empty())
        .collect();
    assert!(leftovers.is_empty(), "the retained message should be gone: {leftovers:?}");
}

#[test]
fn last_will_is_published_when_the_client_vanishes() {
    let broker = TestBroker::start();
    let proxy = TcpProxy::start(broker.v4);
    let mut watcher = Client::connected(&broker.profile());
    let will = LastWill { topic: "test/will".into(), payload: "gone".into(), qos: QoS::AtLeastOnce, retain: false };
    let _doomed = Client::connected(&ConnectionProfile { last_will: Some(will), ..profile_for(proxy.port) });
    proxy.cut();
    assert_eq!(&watcher.wait_message("test/will").payload[..], b"gone");
}

#[test]
fn replay_publishes_steps_with_their_delays() {
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.profile());
    let steps: Vec<ReplayStep> = (0..5_u8)
        .map(|index| ReplayStep {
            delay: Duration::from_millis(if index == 0 { 0 } else { 150 }),
            request: PublishRequest {
                topic: "test/replay".into(),
                payload: Bytes::from(vec![index]),
                ..Default::default()
            },
        })
        .collect();
    let started = std::time::Instant::now();
    client.send(Command::Replay(steps));
    client.wait("replay end", |event| (*event == Event::ReplayFinished { sent: 5, total: 5 }).then_some(()));
    assert!(started.elapsed() >= Duration::from_millis(600), "delays were honoured: {:?}", started.elapsed());
    assert!(client.log.iter().any(|event| matches!(event, Event::ReplayProgress { total: 5, .. })));
    let seen: Vec<u8> = client.wait_messages("test/replay", 5).iter().map(|message| message.payload[0]).collect();
    assert_eq!(seen, [0, 1, 2, 3, 4]);
}

#[test]
fn replay_can_be_stopped_and_replaced() {
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.profile());
    let slow = |count: usize| -> Vec<ReplayStep> {
        (0..count)
            .map(|_| ReplayStep {
                delay: Duration::from_secs(30),
                request: PublishRequest { topic: "test/slow".into(), ..Default::default() },
            })
            .collect()
    };
    client.send(Command::Replay(slow(3)));
    client.send(Command::Replay(slow(2)));
    client.wait("first replay to be replaced", |event| {
        (*event == Event::ReplayFinished { sent: 0, total: 3 }).then_some(())
    });
    client.send(Command::StopReplay);
    client.wait("second replay to stop", |event| (*event == Event::ReplayFinished { sent: 0, total: 2 }).then_some(()));

    // Disconnecting ends a running replay as well.
    client.send(Command::Replay(slow(4)));
    client.send(Command::Disconnect);
    client.wait("replay to end on disconnect", |event| {
        (*event == Event::ReplayFinished { sent: 0, total: 4 }).then_some(())
    });
    client.wait_state(&ConnectionState::Disconnected);
}

#[test]
fn a_burst_of_messages_arrives_complete_and_in_order() {
    const COUNT: u32 = 5000;
    let broker = TestBroker::start();
    let mut client = Client::connected(&broker.profile());
    for index in 0..COUNT {
        client.send(Command::Publish(PublishRequest {
            topic: "test/burst".into(),
            payload: Bytes::from(index.to_be_bytes().to_vec()),
            ..Default::default()
        }));
    }
    let received = client.wait_messages("test/burst", COUNT as usize);
    let numbers: Vec<u32> = received.iter().map(|m| u32::from_be_bytes(m.payload[..4].try_into().unwrap())).collect();
    assert_eq!(numbers, (0..COUNT).collect::<Vec<_>>());
    let batches = client.log.iter().filter(|event| matches!(event, Event::Messages(_))).count();
    assert!(batches < COUNT as usize / 4, "messages should be batched, got {batches} batches");
}

#[test]
fn dropping_the_handle_stops_the_worker() {
    let broker = TestBroker::start();
    let client = Client::connected(&broker.profile());
    let started = std::time::Instant::now();
    drop(client);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn system_subscriptions_are_given_up_when_the_broker_hangs_up_on_them() {
    // The test broker hangs up on `$SYS` subscriptions, like brokers that forbid them. That is the default
    // profile's second subscription, so the default must still end up connected.
    let broker = TestBroker::start();
    let mut client = Client::new();
    let profile = ConnectionProfile { subscriptions: ConnectionProfile::default().subscriptions, ..broker.profile() };
    client.send(Command::Connect(Box::new(profile)));
    let notice = client.wait_error();
    assert!(notice.contains("$SYS/#") && notice.contains("Continuing without"), "{notice}");
    let attempts: Vec<u32> = client
        .log
        .iter()
        .filter_map(|event| {
            if let Event::State(ConnectionState::Reconnecting { attempt, .. }) = event { Some(*attempt) } else { None }
        })
        .collect();
    assert_eq!(attempts, [1], "attempts keep counting although each connect succeeds: {:?}", client.log);

    // The next attempt sticks, and the remaining subscription (`#`) delivers.
    client.send(publish("any/topic", "still here"));
    assert_eq!(&client.wait_message("any/topic").payload[..], b"still here");
    let later_drops = client
        .log
        .iter()
        .skip_while(|event| !matches!(event, Event::Error(_)))
        .filter(|event| matches!(event, Event::State(ConnectionState::Reconnecting { attempt, .. }) if *attempt > 2))
        .count();
    assert_eq!(later_drops, 0, "{:?}", client.log);
}

#[test]
fn a_broker_that_keeps_dropping_us_is_backed_off_and_explained() {
    // Accepts every connection (CONNACK, return code 0) and hangs up immediately.
    let port = refusing_broker(&[0x20, 0x02, 0x00, 0x00]);
    let mut client = Client::new();
    client.send(Command::Connect(Box::new(ConnectionProfile { subscriptions: Vec::new(), ..profile_for(port) })));
    let hint = client.wait_error();
    assert!(hint.contains("keeps closing the connection"), "{hint}");
    let attempts: Vec<u32> = client
        .log
        .iter()
        .filter_map(|event| {
            if let Event::State(ConnectionState::Reconnecting { attempt, .. }) = event { Some(*attempt) } else { None }
        })
        .collect();
    assert_eq!(attempts, [1], "the back-off grows instead of restarting: {:?}", client.log);
    client.wait("the back-off to keep growing", |event| {
        matches!(event, Event::State(ConnectionState::Reconnecting { attempt: 3, .. })).then_some(())
    });
}
