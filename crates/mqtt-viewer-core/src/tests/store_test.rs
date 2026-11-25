use bytes::Bytes;
use jiff::{SignedDuration, Timestamp};

use crate::message::{Message, Properties, QoS};
use crate::store::{Retention, Store, StoreWriter};

fn message(topic: &str, payload: &str, age_days: i64) -> Message {
    Message {
        received_at: now() - SignedDuration::from_hours(24 * age_days),
        ..Message::new(topic, payload.to_owned())
    }
}

fn now() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

#[test]
fn messages_survive_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep").join("history.sqlite");
    let rich = Message {
        qos: QoS::ExactlyOnce,
        retain: true,
        properties: Some(Box::new(Properties {
            user_properties: vec![("k".into(), "v".into())],
            ..Default::default()
        })),
        payload: Bytes::from_static(b"\x00\xff"),
        ..message("a/b", "", 0)
    };
    {
        let mut store = Store::open(&path).unwrap();
        store.insert("p1", &[message("a/a", "1", 0), rich.clone()]).unwrap();
        store.insert("p2", &[message("z", "other profile", 0)]).unwrap();
        assert_eq!(store.count().unwrap(), 3);
        assert!(store.stored_bytes().unwrap() > 0);
    }
    let store = Store::open(&path).unwrap();
    let loaded = store.load_recent("p1", 10).unwrap();
    assert_eq!(loaded, [message("a/a", "1", 0), rich]);
    assert_eq!(store.load_recent("p2", 10).unwrap().len(), 1);
    assert_eq!(store.load_recent("unknown", 10).unwrap().len(), 0);
}

#[test]
fn load_is_limited_per_topic_and_ordered_by_arrival() {
    let mut store = Store::in_memory().unwrap();
    let mut batch = Vec::new();
    for index in 0..5 {
        batch.push(message("a", &format!("a{index}"), 0));
        batch.push(message("b", &format!("b{index}"), 0));
    }
    store.insert("p", &batch).unwrap();
    let payloads: Vec<String> =
        store.load_recent("p", 2).unwrap().iter().map(|m| String::from_utf8_lossy(&m.payload).into_owned()).collect();
    assert_eq!(payloads, ["a3", "b3", "a4", "b4"]);
    assert_eq!(store.load_recent("p", 0).unwrap().len(), 0);
    assert_eq!(store.load_recent("p", usize::MAX).unwrap().len(), 10);
}

#[test]
fn prune_applies_the_age_limit() {
    let mut store = Store::in_memory().unwrap();
    store.insert("p", &[message("t", "old", 10), message("t", "edge", 6), message("t", "new", 0)]).unwrap();
    let deleted = store.prune(Retention { max_age_days: 7, max_bytes: 0 }, now()).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(store.count().unwrap(), 2);
    assert_eq!(store.prune(Retention { max_age_days: 0, max_bytes: 0 }, now()).unwrap(), 0, "0 means unlimited");
}

#[test]
fn prune_applies_the_size_limit_oldest_first() {
    let mut store = Store::in_memory().unwrap();
    let payload = "x".repeat(1000);
    let batch: Vec<Message> = (0..10).map(|index| message(&format!("t{index}"), &payload, 0)).collect();
    store.insert("p", &batch).unwrap();
    let before = store.stored_bytes().unwrap();
    assert!(before > 10_000);
    let deleted = store.prune(Retention { max_age_days: 0, max_bytes: 3_500 }, now()).unwrap();
    assert_eq!(deleted, 7);
    assert!(store.stored_bytes().unwrap() <= 3_500);
    let topics: Vec<String> = store.load_recent("p", 1).unwrap().into_iter().map(|m| m.topic).collect();
    assert_eq!(topics, ["t7", "t8", "t9"]);
    assert_eq!(store.prune(Retention { max_age_days: 0, max_bytes: 1_000_000 }, now()).unwrap(), 0);
}

#[test]
fn clear_can_target_one_profile() {
    let mut store = Store::in_memory().unwrap();
    store.insert("p1", &[message("a", "1", 0), message("b", "2", 0)]).unwrap();
    store.insert("p2", &[message("a", "3", 0)]).unwrap();
    assert_eq!(store.clear(Some("p1")).unwrap(), 2);
    assert_eq!(store.count().unwrap(), 1);
    assert_eq!(store.clear(None).unwrap(), 1);
    assert_eq!(store.count().unwrap(), 0);
    assert_eq!(store.stored_bytes().unwrap(), 0);
}

#[test]
fn unusable_paths_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain-file");
    std::fs::write(&file, "x").unwrap();
    let error = Store::open(&file.join("history.sqlite")).unwrap_err();
    assert!(error.to_string().contains("history database"), "{error}");
    std::fs::write(dir.path().join("garbage.sqlite"), "this is not a database, not even close to one").unwrap();
    Store::open(&dir.path().join("garbage.sqlite")).unwrap_err();
}

#[test]
fn writer_thread_persists_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let writer = StoreWriter::spawn(Store::open(&path).unwrap()).unwrap();
    let real = |payload: &str, age_days: i64| Message {
        received_at: Timestamp::now() - SignedDuration::from_hours(24 * age_days),
        ..Message::new("a", payload.to_owned())
    };
    writer.insert("p", vec![real("old", 30), real("new", 0)]);
    writer.insert("p", Vec::new());
    writer.insert("q", vec![message("b", "x", 0)]);
    writer.flush();
    assert_eq!(Store::open(&path).unwrap().count().unwrap(), 3);

    writer.prune(Retention { max_age_days: 7, max_bytes: 0 });
    writer.clear(Some("q"));
    writer.flush();
    let reader = Store::open(&path).unwrap();
    assert_eq!(reader.count().unwrap(), 1, "prune uses the real clock, so the 30-day-old message is gone");

    writer.clear(None);
    writer.insert("p", vec![message("late", "queued before drop", 0)]);
    drop(writer);
    assert_eq!(reader.count().unwrap(), 1, "dropping the writer finishes queued work");
}
