//! Interaction tests: the real UI code driven through its accessibility tree, without a window or a broker.

use eframe::egui::{Key, Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use mqtt_viewer_core::connection::{Command, ConnectionState, Event};
use mqtt_viewer_core::tree::TopicTree;

use super::{connect, message, populated, receive, state};
use crate::state::{Action, AppState, Dialog, ValueMode};

fn harness(state: AppState) -> Harness<'static, AppState> {
    let mut harness = Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        .build_ui_state(|ui, state: &mut AppState| crate::ui::root(ui, state), state);
    harness.run_steps(8);
    harness
}

/// Clicks the button with this label. Section headers share labels with buttons ("Publish"), so the role
/// narrows the query; of several equally named buttons the last one (the dialog on top) is meant.
fn click(harness: &mut Harness<'static, AppState>, label: &str) {
    let candidates: Vec<_> = harness.query_all_by_label(label).collect();
    let buttons: Vec<_> = candidates.iter().filter(|node| node.accesskit_node().role() == Role::Button).collect();
    match (buttons.last(), candidates.last()) {
        (Some(button), _) => button.click(),
        (None, Some(other)) => other.click(),
        (None, None) => panic!("nothing is labelled {label:?}"),
    }
    // Dialogs need a few frames to settle after they open.
    harness.run_steps(8);
}

fn input<'a>(harness: &'a Harness<'static, AppState>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(Role::TextInput, label)
}

#[test]
fn the_connect_dialog_saves_and_connects() {
    let mut harness = harness(state());
    click(&mut harness, "Connect…");
    assert!(matches!(harness.state().dialog, Dialog::Connect(_)));
    harness.get_by_label("MQTT connection");
    harness.get_by_label("No saved connections yet");

    input(&harness, "Host").focus();
    harness.run_steps(2);
    harness.key_press_modifiers(eframe::egui::Modifiers::COMMAND, Key::A);
    input(&harness, "Host").type_text("broker.test");
    harness.run_steps(3);
    click(&mut harness, "Save");
    assert_eq!(harness.state().config.profiles[0].host, "broker.test");

    click(&mut harness, "Connect");
    assert_eq!(harness.state().dialog, Dialog::None);
    let commands = harness.state_mut().take_commands();
    assert!(matches!(&commands[..], [Command::Connect(profile)] if profile.host == "broker.test"), "{commands:?}");
}

#[test]
fn the_connect_dialog_manages_profiles_and_cancels() {
    let mut harness = harness(state());
    click(&mut harness, "Connect…");
    click(&mut harness, "Save");
    click(&mut harness, "Duplicate");
    assert_eq!(harness.state().config.profiles.len(), 2);
    harness.get_by_label("New connection (copy)");
    click(&mut harness, "Delete");
    assert_eq!(harness.state().config.profiles.len(), 1);
    click(&mut harness, "New");
    click(&mut harness, "Cancel");
    assert_eq!(harness.state().dialog, Dialog::None);

    click(&mut harness, "Connect…");
    harness.key_press(Key::Escape);
    harness.run_steps(3);
    assert_eq!(harness.state().dialog, Dialog::None, "Escape closes dialogs");
}

#[test]
fn every_profile_section_renders_for_tls_websocket_and_mqtt5() {
    let mut state = state();
    state.apply(Action::OpenConnectDialog);
    if let Dialog::Connect(dialog) = &mut state.dialog {
        dialog.draft.transport = mqtt_viewer_core::config::Transport::Wss;
        dialog.draft.protocol = mqtt_viewer_core::config::ProtocolVersion::V5;
        dialog.draft.validate_certificate = false;
        dialog.draft.ca_file = Some("ca.pem".into());
        dialog.draft.last_will = Some(mqtt_viewer_core::config::LastWill::default());
        dialog.draft.session_expiry_secs = Some(60);
        dialog.draft.host.clear();
    }
    let mut harness = harness(state);
    harness.get_by_label("WebSocket path");
    harness.get_by_label("host must not be empty");
    click(&mut harness, "Session");
    harness.get_by_label("Session expiry (s)");
    // Close it again: sections scrolled out of view are not part of the accessibility tree.
    click(&mut harness, "Session");
    click(&mut harness, "Subscriptions");
    click(&mut harness, "Last will");
    harness.get_by_label("Will topic");
    click(&mut harness, "Last will");
    click(&mut harness, "Subscriptions");
    harness.get_by_label("Any certificate is accepted: the connection can be intercepted.");
    click(&mut harness, "Add subscription");
    let Dialog::Connect(dialog) = &harness.state().dialog else { panic!("dialog closed") };
    assert_eq!(dialog.draft.subscriptions.len(), 3);
}

#[test]
fn rows_select_expand_and_follow_the_keyboard() {
    let mut harness = harness(populated());
    click(&mut harness, "home");
    assert_eq!(harness.state().selected_topic().as_deref(), Some("home"));
    harness.key_press(Key::ArrowRight);
    harness.run_steps(3);
    harness.key_press(Key::ArrowDown);
    harness.run_steps(3);
    assert_eq!(harness.state().selected_topic().as_deref(), Some("home/bedroom"));
    harness.key_press(Key::End);
    harness.run_steps(3);
    assert_eq!(harness.state().selected_topic().as_deref(), Some("home/livingroom"));
    harness.key_press(Key::Home);
    harness.run_steps(3);
    assert_eq!(harness.state().selected_topic().as_deref(), Some("$SYS"));

    harness.key_press(Key::Space);
    harness.run_steps(3);
    assert!(harness.state().paused, "Space pauses");
    click(&mut harness, "Resume (0)");
    assert!(!harness.state().paused);
}

#[test]
fn the_sidebar_shows_the_selected_value_in_all_modes() {
    let mut state = populated();
    state.apply(Action::SetExpandedRecursive(TopicTree::ROOT, true));
    let mut harness = harness(state);
    harness.get_by_label("Select a topic in the tree");
    click(&mut harness, "kitchen");
    harness.get_by_label("No message on this exact topic; pick one of its subtopics");

    let climate = harness.state().tree.find("home/kitchen/climate");
    harness.state_mut().apply(Action::Select(climate));
    harness.run_steps(3);
    harness.get_by_label("QoS 0");
    click(&mut harness, "Raw");
    assert_eq!(harness.state().value_mode, ValueMode::Raw);
    click(&mut harness, "Diff");
    harness.get_by_label("Compared with the previous message");
    click(&mut harness, "Decoded");

    click(&mut harness, "/temperature");
    assert_eq!(harness.state().charts.len(), 1);
    harness.get_by_label("home/kitchen/climate /temperature");
    click(&mut harness, "Steps");
    assert!(harness.state().charts[0].stepped);
    click(&mut harness, "Fixed range");
    assert!(harness.state().charts[0].y_range.is_some());
    harness.get_by_label("Range minimum");

    click(&mut harness, "History");
    harness.get_by_label("30 stored of 30 received");
    // Make room: sections scrolled out of view are not part of the accessibility tree.
    for section in ["History", "Value"] {
        click(&mut harness, section);
    }
    click(&mut harness, "Stats");
    harness.get_by_label("Selected: messages");
    click(&mut harness, "Stats");
    click(&mut harness, "Value");

    click(&mut harness, "Copy topic");
    click(&mut harness, "Delete");
    assert_eq!(harness.state().tree.find("home/kitchen/climate"), None);
    let commands = harness.state_mut().take_commands();
    assert!(matches!(&commands[..], [Command::Publish(request)] if request.retain && request.payload.is_empty()));
}

#[test]
fn binary_values_and_mqtt5_properties_render() {
    let mut state = state();
    let mut with_properties = message("v5/topic", "payload", 0);
    with_properties.retain = true;
    with_properties.properties = Some(Box::new(mqtt_viewer_core::message::Properties {
        user_properties: vec![("origin".into(), "test".into())],
        content_type: Some("text/plain".into()),
        response_topic: Some("reply/here".into()),
        correlation_data: Some("id-7".into()),
        message_expiry_secs: Some(30),
        payload_is_utf8: Some(true),
    }));
    state.handle_event(Event::Messages(vec![with_properties, message("bin", "\u{1}\u{2}", 1)]));
    state.apply(Action::SetExpandedRecursive(TopicTree::ROOT, true));
    let node = state.tree.find("v5/topic");
    state.apply(Action::Select(node));
    let mut harness = harness(state);
    harness.get_by_label("retained");
    harness.get_by_label("Content type");
    harness.get_by_label("reply/here");
    harness.get_by_label("origin");

    let binary = harness.state().tree.find("bin");
    harness.state_mut().apply(Action::Select(binary));
    harness
        .state_mut()
        .apply(Action::SetDecoder { pattern: "bin".into(), format: mqtt_viewer_core::decode::Format::Json });
    harness.run_steps(3);
    harness.get_by_label_contains("Cannot decode as JSON");
}

#[test]
fn search_filters_the_tree() {
    let mut harness = harness(populated());
    input(&harness, "Search topics").focus();
    harness.run_steps(2);
    input(&harness, "Search topics").type_text("garage");
    harness.run_steps(3);
    assert_eq!(harness.state().search_text, "garage");
    harness.get_by_label("garage");
    assert!(harness.query_by_label("$SYS").is_none(), "non-matching branches are hidden");

    harness.state_mut().search_text = "nothing matches this".into();
    harness.state_mut().apply(Action::SearchChanged);
    harness.run_steps(3);
    harness.get_by_label("No topic matches the search");
}

#[test]
fn empty_states_explain_themselves() {
    let mut idle = harness(state());
    idle.get_by_label("Connect to a broker to see its topics");
    idle.get_by_label("Not connected");
    idle.state_mut().handle_event(Event::State(ConnectionState::Connecting));
    idle.run_steps(3);
    idle.get_by_label("Waiting for messages…");
    idle.state_mut().handle_event(Event::State(ConnectionState::Reconnecting { attempt: 3, error: "refused".into() }));
    idle.run_steps(3);
    idle.get_by_label("Reconnecting, attempt 3: refused");
}

#[test]
fn publishing_from_the_panel() {
    let mut state = state();
    connect(&mut state);
    let mut harness = harness(state);
    input(&harness, "Publish topic").focus();
    harness.run_steps(2);
    input(&harness, "Publish topic").type_text("cmd/light");
    harness.run_steps(2);
    harness.get_by_label("Publish payload").focus();
    harness.run_steps(2);
    harness.get_by_label("Publish payload").type_text("{\"on\":true}");
    harness.run_steps(2);
    click(&mut harness, "json");
    click(&mut harness, "Format");
    assert_eq!(harness.state().publish.payload, "{\n  \"on\": true\n}");
    click(&mut harness, "retain");
    click(&mut harness, "Publish");
    let commands = harness.state_mut().take_commands();
    assert!(
        matches!(&commands[..], [Command::Publish(request)] if request.topic == "cmd/light" && request.retain),
        "{commands:?}"
    );

    harness.state_mut().publish.payload = "{".into();
    harness.run_steps(3);
    harness.get_by_label_contains("invalid JSON");
    click(&mut harness, "MQTT 5 properties");
    click(&mut harness, "Add property");
    assert_eq!(harness.state().publish.user_properties.len(), 1);
    harness.get_by_label("Property key");
}

#[test]
fn settings_can_be_changed_and_saved() {
    let mut harness = harness(populated());
    click(&mut harness, "Settings");
    assert!(matches!(harness.state().dialog, Dialog::Settings(_)));
    click(&mut harness, "Dark");
    click(&mut harness, "Most topics");
    click(&mut harness, "Store received messages in a local database");
    click(&mut harness, "Save");
    let settings = &harness.state().config.settings;
    assert_eq!(settings.theme, mqtt_viewer_core::config::Theme::Dark);
    assert_eq!(settings.tree_order, mqtt_viewer_core::config::TreeOrder::Topics);
    assert!(settings.persist_history);
    assert_eq!(harness.state().dialog, Dialog::None);

    click(&mut harness, "Settings");
    click(&mut harness, "Clear stored history");
    click(&mut harness, "Cancel");
    assert_eq!(harness.state().dialog, Dialog::None);
}

#[test]
fn deleting_a_subtree_asks_for_confirmation() {
    let mut state = populated();
    let home = state.tree.find("home");
    state.apply(Action::Select(home));
    let mut harness = harness(state);
    click(&mut harness, "Delete all below…");
    harness.get_by_label("Delete retained messages");
    harness.get_by_label("This clears the retained message of 9 topics on the broker:");
    click(&mut harness, "Cancel");
    assert!(harness.state().tree.find("home").is_some());

    click(&mut harness, "Delete all below…");
    click(&mut harness, "Delete");
    assert_eq!(harness.state().tree.find("home"), None);
    assert_eq!(harness.state_mut().take_commands().len(), 9);
}

#[test]
fn the_replay_dialog_shows_the_target_and_starts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut state = state();
    state.apply(Action::StartRecording(path.clone()));
    receive(&mut state, &[("a", "1"), ("b", "2")]);
    state.apply(Action::StopRecording);
    state.apply(Action::LoadRecording(path));
    let mut harness = harness(state);
    harness.get_by_label("Replay recording");
    harness.get_by_label("Not connected to a broker");

    connect(harness.state_mut());
    harness.state_mut().dialog = Dialog::None;
    let recording = dir.path().join("session.jsonl");
    harness.state_mut().apply(Action::LoadRecording(recording));
    harness.run_steps(3);
    harness.get_by_label("The messages will be published to mqtt://localhost:1883");
    click(&mut harness, "Start replay");
    assert!(matches!(harness.state_mut().take_commands()[..], [Command::Replay(_)]));
    harness.get_by_label("Stop replay 0/2");
    click(&mut harness, "Stop replay 0/2");
    assert!(matches!(harness.state_mut().take_commands()[..], [Command::StopReplay]));
}

#[test]
fn toolbar_actions_and_notices() {
    let mut state = populated();
    state.info("Something happened");
    state.error("Something broke");
    let mut harness = harness(state);
    harness.get_by_label("Something happened");
    harness.get_by_label("Something broke");
    harness.get_all_by_label("×").next().unwrap().click();
    harness.run_steps(3);
    assert_eq!(harness.state().notices.len(), 1);

    click(&mut harness, "Pause");
    assert!(harness.state().paused);
    click(&mut harness, "Clear");
    assert_eq!(harness.state().tree.stats().topics, 0);
    click(&mut harness, "Disconnect");
    assert!(matches!(harness.state_mut().take_commands()[..], [Command::Disconnect]));
}
