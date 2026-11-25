//! The whole application against a real broker, driven through its window.

use std::time::{Duration, Instant};

use eframe::egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use mqtt_viewer_core::config::{AppConfig, Subscription};
use mqtt_viewer_core::connection::ConnectionState;
use mqtt_viewer_core::diff::ChangeKind;
use mqtt_viewer_core::message::QoS;

use super::broker;
use crate::app::App;
use crate::state::{Action, AppState, Dialog};

fn harness(state: AppState) -> Harness<'static, App> {
    Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        .build_eframe(move |creation| App::with_state(&creation.egui_ctx, state).unwrap())
}

/// Runs frames until `done` holds, failing after a generous timeout.
fn run_until(harness: &mut Harness<'static, App>, what: &str, done: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.run_steps(2);
        if done(harness.state().state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; notices: {:?}",
            harness.state().state().notices
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn first_start_opens_the_connection_dialog() {
    let harness = harness(AppState::new(AppConfig::default(), None, None));
    assert!(matches!(harness.state().state().dialog, Dialog::Connect(_)));
    harness.get_by_label("MQTT connection");
}

#[test]
fn connect_browse_and_publish_through_the_window() {
    let port = broker::start();
    let mut harness = harness(AppState::new(AppConfig::default(), None, None));
    harness.run_steps(8);

    // Point the default profile at the test broker and connect with the dialog's button.
    if let Dialog::Connect(dialog) = &mut harness.state_mut().state_mut().dialog {
        dialog.draft.host = "127.0.0.1".into();
        dialog.draft.port = port;
        dialog.draft.name = "Test broker".into();
        // The subscriptions stay at the default (`#` and `$SYS/#`). This broker hangs up on `$SYS`
        // subscriptions, so the test also proves that the default profile recovers from that on its own.
    }
    harness.run_steps(8);
    harness.get_by_label("Connect").click();
    run_until(&mut harness, "the `$SYS` subscription to be given up", |state| {
        state.connection == ConnectionState::Connected
            && state.notices.iter().any(|notice| notice.text.contains("$SYS/#"))
    });
    harness.get_by_label("Test broker");

    // Publish through the publish panel; the message comes back through the subscription to `#`.
    {
        let form = &mut harness.state_mut().state_mut().publish;
        form.topic = "greenhouse/bed1/moisture".into();
        form.payload = "41".into();
    }
    harness.run_steps(2);
    harness.get_by_role_and_label(eframe::egui::Role::Button, "Publish").click();
    run_until(&mut harness, "the published message", |state| state.tree.find("greenhouse/bed1/moisture").is_some());

    // The new topic shows up as a collapsed top-level row; expand down to it and select it.
    for level in ["greenhouse", "bed1"] {
        harness.get_by_label(level).click();
        harness.run_steps(2);
        harness.key_press(eframe::egui::Key::ArrowRight);
        harness.run_steps(2);
    }
    harness.get_by_label("moisture").click();
    harness.run_steps(2);
    assert_eq!(harness.state().state().selected_topic().as_deref(), Some("greenhouse/bed1/moisture"));
    harness.get_by_label("Value");

    harness.get_by_label("Disconnect").click();
    run_until(&mut harness, "the disconnect", |state| state.connection == ConnectionState::Disconnected);
    harness.get_by_label("Connect…");
}

/// A harness connected to `port` with a QoS 2 subscription to everything.
fn connected(port: u16, state: AppState) -> Harness<'static, App> {
    let mut harness = harness(state);
    harness.state_mut().state_mut().apply(Action::NewProfile);
    if let Dialog::Connect(dialog) = &mut harness.state_mut().state_mut().dialog {
        dialog.draft.id = "test-profile".into();
        dialog.draft.host = "127.0.0.1".into();
        dialog.draft.port = port;
        dialog.draft.subscriptions = vec![Subscription { filter: "#".into(), qos: QoS::ExactlyOnce }];
    }
    harness.state_mut().state_mut().apply(Action::Connect);
    run_until(&mut harness, "the connection", |state| state.connection == ConnectionState::Connected);
    harness
}

fn publish(harness: &mut Harness<'static, App>, topic: &str, payload: &str, retain: bool) {
    let form = &mut harness.state_mut().state_mut().publish;
    form.topic = topic.into();
    form.payload = payload.into();
    form.retain = retain;
    harness.state_mut().state_mut().apply(Action::Publish);
}

fn message_count(state: &AppState, topic: &str) -> u64 {
    state.tree.find(topic).map_or(0, |node| state.tree.node(node).message_count())
}

#[test]
fn live_values_feed_charts_and_diffs_and_retained_topics_can_be_deleted() {
    let port = broker::start();
    let mut harness = connected(port, AppState::new(AppConfig::default(), None, None));
    publish(&mut harness, "plant/temp", r#"{"t":21.0}"#, true);
    run_until(&mut harness, "the first value", |state| message_count(state, "plant/temp") == 1);
    publish(&mut harness, "plant/temp", r#"{"t":22.5}"#, true);
    run_until(&mut harness, "the second value", |state| message_count(state, "plant/temp") == 2);

    let app = harness.state_mut().state_mut();
    let node = app.tree.find("plant/temp");
    app.apply(Action::Select(node));
    app.apply(Action::AddChart { topic: "plant/temp".into(), pointer: "/t".into() });
    assert_eq!(app.charts[0].points.iter().map(|point| point[1]).collect::<Vec<_>>(), [21.0, 22.5]);
    let changed = app.selected_diff().unwrap().iter().filter(|line| line.kind != ChangeKind::Equal).count();
    assert_eq!(changed, 2, "one line removed, one added");

    publish(&mut harness, "plant/temp", r#"{"t":23.0}"#, true);
    run_until(&mut harness, "the chart to follow", |state| state.charts[0].latest() == Some(23.0));

    // Delete the retained message and check with a second session that the broker forgot it.
    let app = harness.state_mut().state_mut();
    let node = app.tree.find("plant/temp").unwrap();
    app.apply(Action::DeleteRetained { node, recursive: false });
    run_until(&mut harness, "the topic to disappear", |state| state.tree.find("plant/temp").is_none());
    publish(&mut harness, "sync/marker", "1", false);
    run_until(&mut harness, "the marker", |state| state.tree.find("sync/marker").is_some());
    assert_eq!(harness.state().state().tree.find("plant/temp"), None, "the broker's echo does not bring it back");

    let mut second = connected(port, AppState::new(AppConfig::default(), None, None));
    publish(&mut second, "sync/marker2", "1", false);
    run_until(&mut second, "the second session's marker", |state| state.tree.find("sync/marker2").is_some());
    assert_eq!(second.state().state().tree.find("plant/temp"), None, "no retained message is delivered any more");
}

#[test]
fn stored_history_survives_a_restart() {
    let port = broker::start();
    let directory = tempfile::tempdir().unwrap();
    let (config, database) = (directory.path().join("config.toml"), directory.path().join("history.sqlite"));
    let mut settings = AppConfig::default();
    settings.settings.persist_history = true;
    settings.save(&config).unwrap();

    let mut first = connected(port, AppState::load_from(Some(config.clone()), Some(database.clone())));
    assert!(first.state().state().is_persisting());
    for value in ["1", "2", "3"] {
        publish(&mut first, "meter/energy", value, false);
    }
    run_until(&mut first, "the readings", |state| message_count(state, "meter/energy") == 3);
    drop(first);

    // A new session: the readings are back from disk before the broker has sent anything.
    let mut second = harness(AppState::load_from(Some(config), Some(database)));
    let app = second.state_mut().state_mut();
    app.apply(Action::OpenConnectDialog);
    app.apply(Action::Connect);
    let node = app.tree.find("meter/energy").expect("the stored topic is restored on connect");
    let payloads: Vec<&[u8]> = app.tree.node(node).history().iter().map(|entry| &entry.payload[..]).collect();
    assert_eq!(payloads, [b"1", b"2", b"3"]);
    run_until(&mut second, "the connection", |state| state.connection == ConnectionState::Connected);
}
