//! Record a session from the broker, replay it under another prefix and compare what arrives.

use mqtt_viewer_core::config::{ConnectionProfile, Subscription};
use mqtt_viewer_core::connection::{Command, Event};
use mqtt_viewer_core::message::{PublishRequest, QoS};
use mqtt_viewer_core::record::{Recorder, ReplayOptions, plan_replay, read_recording};

use crate::common::{Client, TcpProxy, TestBroker, profile_for};

fn subscribed(broker: &TestBroker, filter: &str) -> Client {
    let subscription = Subscription { filter: filter.into(), qos: QoS::ExactlyOnce };
    Client::connected(&ConnectionProfile { subscriptions: vec![subscription], ..broker.profile() })
}

#[test]
fn a_recorded_session_replays_with_topics_payloads_qos_and_retain_intact() {
    let broker = TestBroker::start();
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("session.jsonl");
    let mut publisher = subscribed(&broker, "sync/#");
    let publish = |client: &Client, topic: &str, payload: &'static [u8], qos: QoS, retain: bool| {
        client.send(Command::Publish(PublishRequest {
            topic: topic.into(),
            payload: payload.into(),
            qos,
            retain,
            ..Default::default()
        }));
    };

    // A retained message first, so the recorder receives it flagged as retained when it subscribes.
    publish(&publisher, "rt/state", b"\x00\xffbinary", QoS::ExactlyOnce, true);
    publish(&publisher, "sync/1", b"", QoS::AtLeastOnce, false);
    publisher.wait_message("sync/1");

    let mut recorder_client = subscribed(&broker, "rt/#");
    let retained = recorder_client.wait_message("rt/state");
    assert!(retained.retain);
    publish(&publisher, "rt/a", b"plain", QoS::AtMostOnce, false);
    publish(&publisher, "rt/b/c", br#"{"n":1}"#, QoS::AtLeastOnce, false);
    let live = [recorder_client.wait_message("rt/a"), recorder_client.wait_message("rt/b/c")];

    let mut recorder = Recorder::create(&file).unwrap();
    for message in std::iter::once(&retained).chain(&live) {
        recorder.write(message).unwrap();
    }
    assert_eq!(recorder.finish().unwrap(), 3);

    // Replay under another prefix, through a proxy that shows what goes over the wire.
    let proxy = TcpProxy::start(broker.v4);
    let mut replayer = Client::connected(&ConnectionProfile { subscriptions: Vec::new(), ..profile_for(proxy.port) });
    let mut observer = subscribed(&broker, "replayed/#");
    let recording = read_recording(&file).unwrap();
    let options = ReplayOptions { speed: 0.0, prefix_from: "rt/".into(), prefix_to: "replayed/".into() };
    replayer.send(Command::Replay(plan_replay(&recording, &options)));
    replayer
        .wait("the replay to finish", |event| (*event == Event::ReplayFinished { sent: 3, total: 3 }).then_some(()));

    for (topic, payload) in
        [("replayed/state", &b"\x00\xffbinary"[..]), ("replayed/a", b"plain"), ("replayed/b/c", br#"{"n":1}"#)]
    {
        assert_eq!(&observer.wait_message(topic).payload[..], payload, "{topic}");
    }
    // QoS and retain exactly as recorded. (This broker delivers at the subscription's QoS, so the recorder saw
    // every message at QoS 2; what matters is that the replay sends what the recording says.)
    let recorded: Vec<(String, u8, bool)> = recording
        .iter()
        .map(|message| (message.topic.replace("rt/", "replayed/"), message.qos.level(), message.retain))
        .collect();
    assert_eq!(proxy.published(), recorded);
    assert_eq!(recorded[0], ("replayed/state".to_owned(), 2, true));
    assert!(!recorded[1].2 && !recorded[2].2, "live messages are not retained");

    // Only the message recorded as retained is retained again.
    let mut late = subscribed(&broker, "replayed/#");
    let again = late.wait_message("replayed/state");
    assert!(again.retain);
    publish(&publisher, "replayed/marker", b"", QoS::AtMostOnce, false);
    late.wait_message("replayed/marker");
    let others = late.log.iter().filter(|event| matches!(event, Event::Messages(batch) if batch.iter().any(|m| m.topic == "replayed/a" || m.topic == "replayed/b/c"))).count();
    assert_eq!(others, 0, "non-retained messages are not delivered to a late subscriber");
}
