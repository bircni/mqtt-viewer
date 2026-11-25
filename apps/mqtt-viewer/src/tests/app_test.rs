//! The whole application against a real broker, driven through its window.

use std::time::{Duration, Instant};

use eframe::egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use mqtt_viewer_core::config::AppConfig;
use mqtt_viewer_core::connection::ConnectionState;

use super::broker;
use crate::app::App;
use crate::state::{AppState, Dialog};

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
        // The in-process broker hangs up on `$SYS` subscriptions.
        dialog.draft.subscriptions = vec![mqtt_viewer_core::config::Subscription::new("#")];
    }
    harness.run_steps(8);
    harness.get_by_label("Connect").click();
    run_until(&mut harness, "the connection", |state| state.connection == ConnectionState::Connected);
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
