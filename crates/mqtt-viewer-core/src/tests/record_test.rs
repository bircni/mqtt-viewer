use std::time::Duration;

use bytes::Bytes;
use jiff::Timestamp;

use crate::message::{Message, Properties, QoS};
use crate::record::{
    RecordError, RecordedMessage, Recorder, ReplayOptions, export_csv, export_json, plan_replay, read_recording,
    replay_duration,
};

fn at(seconds: i64, millis: i64) -> Timestamp {
    Timestamp::from_millisecond(seconds * 1000 + millis).unwrap()
}

fn sample() -> Vec<Message> {
    vec![
        Message { received_at: at(1_700_000_000, 0), ..Message::new("home/temp", "21.5") },
        Message {
            received_at: at(1_700_000_001, 500),
            qos: QoS::AtLeastOnce,
            retain: true,
            properties: Some(Box::new(Properties { content_type: Some("text/csv".into()), ..Properties::default() })),
            ..Message::new("home/log", "a,\"b\"\nc")
        },
        Message {
            received_at: at(1_700_000_003, 500),
            qos: QoS::ExactlyOnce,
            ..Message::new("home/raw", Bytes::from_static(b"\x00\xff"))
        },
    ]
}

fn recorded() -> Vec<RecordedMessage> {
    sample().iter().map(RecordedMessage::from).collect()
}

#[test]
fn recording_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut recorder = Recorder::create(&path).unwrap();
    assert_eq!(recorder.path(), path);
    for message in sample() {
        recorder.write(&message).unwrap();
    }
    assert_eq!(recorder.count(), 3);
    assert_eq!(recorder.finish().unwrap(), 3);

    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert_eq!(read_recording(&path).unwrap(), recorded());

    // Blank lines (e.g. from manual editing) are tolerated.
    std::fs::write(&path, format!("\n{text}\n\n")).unwrap();
    assert_eq!(read_recording(&path).unwrap().len(), 3);
}

#[test]
fn recording_errors_name_the_file_and_line() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.jsonl");
    let error = read_recording(&missing).unwrap_err();
    assert!(matches!(error, RecordError::Io { .. }), "{error}");
    assert!(error.to_string().contains("missing.jsonl"), "{error}");

    let broken = dir.path().join("broken.jsonl");
    let good = serde_json::to_string(&recorded()[0]).unwrap();
    std::fs::write(&broken, format!("{good}\n{{\"topic\": 3}}\n")).unwrap();
    let error = read_recording(&broken).unwrap_err();
    assert!(matches!(error, RecordError::Parse { line: 2, .. }), "{error}");
    assert!(error.to_string().contains("line 2"), "{error}");

    assert!(Recorder::create(dir.path()).is_err(), "a directory is not a file");
}

#[test]
fn json_export_is_readable() {
    insta::assert_snapshot!(export_json(&recorded()), @r#"
    [
      {
        "timestamp": "2023-11-14T22:13:20Z",
        "topic": "home/temp",
        "qos": 0,
        "retain": false,
        "payload": "21.5"
      },
      {
        "timestamp": "2023-11-14T22:13:21.5Z",
        "topic": "home/log",
        "qos": 1,
        "retain": true,
        "payload": "a,\"b\"\nc",
        "properties": {
          "content_type": "text/csv"
        }
      },
      {
        "timestamp": "2023-11-14T22:13:23.5Z",
        "topic": "home/raw",
        "qos": 2,
        "retain": false,
        "payload_base64": "AP8="
      }
    ]
    "#);
    assert_eq!(export_json(&[]), "[]");
}

#[test]
fn csv_export_quotes_and_flags_binary() {
    insta::assert_snapshot!(export_csv(&recorded()), @r#"
    timestamp,topic,qos,retain,encoding,payload
    2023-11-14T22:13:20Z,home/temp,0,false,text,21.5
    2023-11-14T22:13:21.5Z,home/log,1,true,text,"a,""b""
    c"
    2023-11-14T22:13:23.5Z,home/raw,2,false,base64,AP8=
    "#);
}

#[test]
fn replay_keeps_or_scales_the_original_pacing() {
    let steps = plan_replay(&recorded(), &ReplayOptions::default());
    let delays: Vec<u64> = steps.iter().map(|step| u64::try_from(step.delay.as_millis()).unwrap()).collect();
    assert_eq!(delays, [0, 1500, 2000]);
    assert_eq!(replay_duration(&steps), Duration::from_millis(3500));
    assert_eq!(steps[1].request.topic, "home/log");
    assert_eq!(steps[1].request.qos, QoS::AtLeastOnce);
    assert!(steps[1].request.retain);
    assert_eq!(steps[1].request.properties.content_type.as_deref(), Some("text/csv"));
    assert_eq!(&steps[2].request.payload[..], b"\x00\xff");

    let fast = plan_replay(&recorded(), &ReplayOptions { speed: 2.0, ..ReplayOptions::default() });
    assert_eq!(replay_duration(&fast), Duration::from_millis(1750));
    for speed in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let instant = plan_replay(&recorded(), &ReplayOptions { speed, ..ReplayOptions::default() });
        assert_eq!(replay_duration(&instant), Duration::ZERO, "speed {speed}");
    }
}

#[test]
fn replay_tolerates_out_of_order_timestamps() {
    let mut messages = recorded();
    messages.swap(0, 2);
    let steps = plan_replay(&messages, &ReplayOptions::default());
    assert!(steps.iter().all(|step| step.delay <= Duration::from_secs(4)));
    assert_eq!(steps[1].delay, Duration::ZERO, "going back in time means no wait");
}

#[test]
fn replay_can_rewrite_the_topic_prefix() {
    let options =
        ReplayOptions { prefix_from: "home/".into(), prefix_to: "sandbox/home/".into(), ..Default::default() };
    let topics: Vec<String> = plan_replay(&recorded(), &options).into_iter().map(|step| step.request.topic).collect();
    assert_eq!(topics, ["sandbox/home/temp", "sandbox/home/log", "sandbox/home/raw"]);
    assert_eq!(plan_replay(&[], &options).len(), 0);
}
