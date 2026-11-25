use std::path::PathBuf;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use mqtt_viewer_core::config::{AppConfig, ConnectionProfile, ProtoBinding, TreeOrder};
use mqtt_viewer_core::connection::{Command, ConnectionState, Event};
use mqtt_viewer_core::decode::Format;
use mqtt_viewer_core::message::{Message, PublishRequest};
use mqtt_viewer_core::store::Store;
use mqtt_viewer_core::tree::TopicTree;

use super::{connect, message, notices, populated, receive, state};
use crate::forms::PayloadMode;
use crate::state::{
    Action, AppState, Dialog, ExportFormat, ExportScope, MAX_HELD, MAX_PUBLISH_HISTORY, MAX_VALUE_TEXT,
    NOTICE_LIFETIME, Navigate, ValueMode,
};

fn draft(state: &mut AppState) -> &mut ConnectionProfile {
    let Dialog::Connect(dialog) = &mut state.dialog else { panic!("expected the connect dialog") };
    &mut dialog.draft
}

fn published(state: &mut AppState) -> Vec<PublishRequest> {
    state
        .take_commands()
        .into_iter()
        .filter_map(|command| if let Command::Publish(request) = command { Some(request) } else { None })
        .collect()
}

fn select(state: &mut AppState, topic: &str) {
    let node = state.tree.find(topic).unwrap();
    state.tree.reveal(node);
    state.apply(Action::Select(Some(node)));
}

#[test]
fn profiles_can_be_created_edited_duplicated_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut state = AppState::new(AppConfig::default(), Some(path.clone()), None);
    state.apply(Action::OpenConnectDialog);
    draft(&mut state).name = "Home".into();
    let home = draft(&mut state).id.clone();
    state.apply(Action::SaveProfile);
    assert_eq!(state.config.profiles.len(), 1);

    // Saving again updates in place.
    draft(&mut state).host = "broker.home".into();
    state.apply(Action::SaveProfile);
    assert_eq!(state.config.profiles.len(), 1);
    assert_eq!(state.config.profiles[0].host, "broker.home");

    state.apply(Action::NewProfile);
    assert_ne!(draft(&mut state).id, home);
    draft(&mut state).name = "Office".into();
    let office = draft(&mut state).id.clone();
    state.apply(Action::SaveProfile);

    state.apply(Action::EditProfile(home.clone()));
    assert_eq!(draft(&mut state).name, "Home");
    state.apply(Action::EditProfile("missing".into()));
    assert_eq!(draft(&mut state).name, "Home");

    state.apply(Action::DuplicateProfile(home.clone()));
    assert_eq!(draft(&mut state).name, "Home (copy)");
    assert_eq!(state.config.profiles.len(), 3);
    state.apply(Action::DuplicateProfile("missing".into()));
    assert_eq!(state.config.profiles.len(), 3);

    // Deleting the profile being edited moves the editor to another one.
    let copy = draft(&mut state).id.clone();
    state.apply(Action::DeleteProfile(copy));
    assert_eq!(draft(&mut state).name, "Home");
    state.apply(Action::DeleteProfile(office));
    assert_eq!(AppConfig::load(&path).unwrap().profiles.len(), 1, "every change is saved");

    state.apply(Action::CloseDialog);
    assert_eq!(state.dialog, Dialog::None);
    // Without the dialog, profile actions that need a draft do nothing.
    state.apply(Action::SaveProfile);
    state.apply(Action::EditProfile(home));
    assert_eq!(state.dialog, Dialog::None);
    assert_eq!(notices(&state), Vec::<String>::new());
}

#[test]
fn invalid_drafts_are_not_saved() {
    let mut state = state();
    state.apply(Action::OpenConnectDialog);
    draft(&mut state).host.clear();
    state.apply(Action::SaveProfile);
    state.apply(Action::Connect);
    assert_eq!(state.config.profiles.len(), 0);
    assert_eq!(state.take_commands().len(), 0);
    assert_eq!(notices(&state), ["!Profile is incomplete: host must not be empty"], "a repeated notice is shown once");
    assert!(matches!(state.dialog, Dialog::Connect(_)));
}

#[test]
fn connecting_saves_the_profile_and_resets_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut state = AppState::new(AppConfig::default(), Some(path.clone()), None);
    receive(&mut state, &[("stale/topic", "x")]);
    state.apply(Action::Select(state.tree.find("stale/topic")));
    state.apply(Action::AddChart { topic: "stale/topic".into(), pointer: String::new() });

    state.apply(Action::OpenConnectDialog);
    draft(&mut state).host = "example.org".into();
    state.apply(Action::Connect);
    let commands = state.take_commands();
    assert!(matches!(&commands[..], [Command::Connect(profile)] if profile.host == "example.org"), "{commands:?}");
    assert_eq!(state.dialog, Dialog::None);
    assert_eq!(state.tree.stats().topics, 0);
    assert_eq!(state.selected, None);
    let saved = AppConfig::load(&path).unwrap();
    assert_eq!(saved.last_profile, state.profile.as_ref().map(|profile| profile.id.clone()));

    // The dialog reopens on the profile used last.
    state.apply(Action::OpenConnectDialog);
    assert_eq!(draft(&mut state).host, "example.org");
    let id = draft(&mut state).id.clone();
    state.apply(Action::DeleteProfile(id));
    assert_eq!(state.config.last_profile, None);

    state.apply(Action::Disconnect);
    assert!(matches!(state.take_commands()[..], [Command::Disconnect]));
}

#[test]
fn connection_events_update_state_and_notify_once_per_outage() {
    let mut state = state();
    state.apply(Action::OpenConnectDialog);
    state.apply(Action::Connect);
    state.handle_event(Event::State(ConnectionState::Connecting));
    assert_eq!(state.connection, ConnectionState::Connecting);
    state.handle_event(Event::State(ConnectionState::Connected));
    assert_eq!(notices(&state), ["Connected to mqtt://localhost:1883"]);
    for attempt in 1..=3 {
        state.handle_event(Event::State(ConnectionState::Reconnecting { attempt, error: "refused".into() }));
    }
    assert_eq!(notices(&state).len(), 2);
    assert_eq!(notices(&state)[1], "!Connection lost: refused");
    state.handle_event(Event::Error("boom".into()));
    assert_eq!(notices(&state)[2], "!boom");

    state.replay = Some((1, 2));
    state.handle_event(Event::State(ConnectionState::Disconnected));
    assert_eq!(state.replay, None, "a replay cannot outlive the connection");
}

#[test]
fn messages_fill_the_tree_unless_paused() {
    let mut state = state();
    receive(&mut state, &[("a/b", "1"), ("a/c", "2")]);
    assert_eq!(state.tree.stats().topics, 2);

    state.apply(Action::TogglePause);
    assert!(state.paused);
    receive(&mut state, &[("a/d", "3"), ("a/b", "4")]);
    assert_eq!(state.tree.stats().topics, 2);
    assert_eq!(state.held_count(), 2);

    state.apply(Action::TogglePause);
    assert_eq!(state.held_count(), 0);
    assert_eq!(state.tree.stats().topics, 3);
    assert_eq!(state.tree.stats().messages, 4);
    assert_eq!(notices(&state), Vec::<String>::new());
}

#[test]
fn the_pause_buffer_is_bounded() {
    let mut state = state();
    state.apply(Action::TogglePause);
    let batch: Vec<Message> = (0..MAX_HELD + 5).map(|index| message("t", &index.to_string(), 0)).collect();
    state.handle_event(Event::Messages(batch));
    assert_eq!(state.held_count(), MAX_HELD);
    assert_eq!(state.held_dropped, 5);
    state.apply(Action::TogglePause);
    assert_eq!(notices(&state), ["!5 messages were dropped while paused"]);
    assert_eq!(state.held_dropped, 0);
    assert_eq!(state.tree.stats().messages, MAX_HELD as u64);
}

#[test]
fn selection_follows_the_keyboard() {
    let mut state = state();
    state.apply(Action::Navigate(Navigate::Down));
    assert_eq!(state.selected, None, "nothing to select in an empty tree");

    receive(&mut state, &[("a/x", "1"), ("a/y", "2"), ("b", "3")]);
    let node = |state: &AppState, topic: &str| state.tree.find(topic);
    state.apply(Action::Navigate(Navigate::Down));
    assert_eq!(state.selected, node(&state, "a"), "the first key press selects the first row");
    assert!(std::mem::take(&mut state.reveal_selection));

    state.apply(Action::Navigate(Navigate::Right));
    assert_eq!(state.selected, node(&state, "a"), "right expands first");
    assert!(state.tree.node(state.tree.find("a").unwrap()).is_expanded());
    state.apply(Action::Navigate(Navigate::Right));
    assert_eq!(state.selected, node(&state, "a/x"), "then enters");
    state.apply(Action::Navigate(Navigate::Right));
    assert_eq!(state.selected, node(&state, "a/x"), "a leaf has nowhere to go");
    state.apply(Action::Navigate(Navigate::Down));
    assert_eq!(state.selected, node(&state, "a/y"));
    state.apply(Action::Navigate(Navigate::Left));
    assert_eq!(state.selected, node(&state, "a"), "left on a leaf goes to the parent");
    state.apply(Action::Navigate(Navigate::Left));
    assert!(!state.tree.node(state.tree.find("a").unwrap()).is_expanded(), "left on an open node collapses it");
    state.apply(Action::Navigate(Navigate::Left));
    assert_eq!(state.selected, node(&state, "a"), "a top-level node has no visible parent");

    state.apply(Action::Navigate(Navigate::Last));
    assert_eq!(state.selected, node(&state, "b"));
    state.apply(Action::Navigate(Navigate::Down));
    assert_eq!(state.selected, node(&state, "b"));
    state.apply(Action::Navigate(Navigate::First));
    state.apply(Action::Navigate(Navigate::Up));
    assert_eq!(state.selected, node(&state, "a"));

    // A selection hidden by a collapsed parent restarts at the top.
    select(&mut state, "a/y");
    state.apply(Action::SetExpandedRecursive(TopicTree::ROOT, false));
    state.apply(Action::Navigate(Navigate::Down));
    assert_eq!(state.selected, node(&state, "a"));
}

#[test]
fn selection_and_expansion_actions() {
    let mut state = state();
    receive(&mut state, &[("a/b/c", "1")]);
    let a = state.tree.find("a").unwrap();
    state.apply(Action::SetExpanded(a, true));
    assert_eq!(state.tree.rows().len(), 2);
    state.apply(Action::SetExpandedRecursive(a, true));
    assert_eq!(state.tree.rows().len(), 3);
    state.apply(Action::Select(Some(a)));
    assert_eq!(state.selected_topic().as_deref(), Some("a"));
    state.apply(Action::Select(None));
    assert_eq!(state.selected_topic(), None);
    state.apply(Action::ClearTree);
    state.apply(Action::Select(Some(a)));
    assert_eq!(state.selected, None, "stale handles are not selectable");
}

#[test]
fn search_and_order_drive_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut state = AppState::new(AppConfig::default(), Some(path.clone()), None);
    receive(&mut state, &[("home/temp", "warm"), ("office/temp", "cold"), ("office/door", "open"), ("office/x", "1")]);
    state.search_text = " door ".into();
    state.apply(Action::SearchChanged);
    assert_eq!(state.tree.rows().len(), 2);
    state.search_text = "cold".into();
    state.apply(Action::SearchChanged);
    assert_eq!(state.tree.rows().len(), 0);
    state.search_payload = true;
    state.apply(Action::SearchChanged);
    assert_eq!(state.tree.rows().len(), 2);
    state.search_text.clear();
    state.apply(Action::SearchChanged);

    state.apply(Action::SetOrder(TreeOrder::Messages));
    let first = state.tree.rows()[0].id;
    assert_eq!(state.tree.topic(first), "office");
    assert_eq!(AppConfig::load(&path).unwrap().settings.tree_order, TreeOrder::Messages);
}

#[test]
fn publishing_validates_and_keeps_a_history() {
    let mut state = state();
    state.apply(Action::Publish);
    assert_eq!(notices(&state), ["!topic must not be empty"]);
    assert_eq!(state.take_commands().len(), 0);

    state.publish.topic = "cmd/light".into();
    state.publish.payload = "ON".into();
    state.apply(Action::Publish);
    state.publish.payload = "OFF".into();
    state.apply(Action::Publish);
    state.publish.payload = "ON".into();
    state.apply(Action::Publish);
    let sent = published(&mut state);
    assert_eq!(sent.len(), 3);
    let history: Vec<&[u8]> = state.publish_history.iter().map(|request| &request.payload[..]).collect();
    assert_eq!(history, [b"ON" as &[u8], b"OFF"], "repeats move to the front instead of piling up");

    state.apply(Action::LoadPublishHistory(1));
    assert_eq!(state.publish.payload, "OFF");
    state.apply(Action::LoadPublishHistory(99));
    assert_eq!(state.publish.payload, "OFF");

    for index in 0..MAX_PUBLISH_HISTORY + 5 {
        state.publish.payload = index.to_string();
        state.apply(Action::Publish);
    }
    assert_eq!(state.publish_history.len(), MAX_PUBLISH_HISTORY);
}

#[test]
fn publish_payload_formatting_and_topic_shortcut() {
    let mut state = state();
    state.publish.mode = PayloadMode::Json;
    state.publish.payload = r#"{"a":1}"#.into();
    state.apply(Action::FormatPublishPayload);
    assert_eq!(state.publish.payload, "{\n  \"a\": 1\n}");
    state.publish.payload = "{".into();
    state.apply(Action::FormatPublishPayload);
    assert_eq!(notices(&state).len(), 1);

    receive(&mut state, &[("a/b", "1")]);
    let node = state.tree.find("a/b").unwrap();
    state.apply(Action::UseTopicForPublish(node));
    assert_eq!(state.publish.topic, "a/b");
    state.apply(Action::ClearTree);
    state.publish.topic = "kept".into();
    state.apply(Action::UseTopicForPublish(node));
    assert_eq!(state.publish.topic, "kept");
}

#[test]
fn presets_are_saved_loaded_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut state = AppState::new(AppConfig::default(), Some(path.clone()), None);
    state.apply(Action::SavePreset("broken".into()));
    assert_eq!(notices(&state), ["!topic must not be empty"]);
    state.publish.topic = "cmd/light".into();
    state.publish.payload = "ON".into();
    state.apply(Action::SavePreset("  ".into()));
    assert_eq!(notices(&state)[1], "!A preset needs a name");
    state.apply(Action::SavePreset(" light on ".into()));
    state.publish.payload = "OFF".into();
    state.apply(Action::SavePreset("light on".into()));
    assert_eq!(state.config.publish_presets.len(), 1, "saving under the same name replaces the preset");
    assert_eq!(AppConfig::load(&path).unwrap().publish_presets[0].name, "light on");

    state.publish = crate::forms::PublishForm::default();
    state.apply(Action::LoadPreset(0));
    assert_eq!((state.publish.topic.as_str(), state.publish.payload.as_str()), ("cmd/light", "OFF"));
    state.apply(Action::LoadPreset(5));
    state.apply(Action::DeletePreset(5));
    state.apply(Action::DeletePreset(0));
    assert_eq!(AppConfig::load(&path).unwrap().publish_presets.len(), 0);
}

#[test]
fn deleting_a_retained_topic_publishes_an_empty_retained_message() {
    let mut state = state();
    receive(&mut state, &[("a/b", "1"), ("a/c", "2")]);
    let node = state.tree.find("a/b").unwrap();
    state.apply(Action::Select(Some(node)));

    state.apply(Action::DeleteRetained { node, recursive: false });
    assert_eq!(notices(&state), ["!Connect to a broker before deleting retained messages"]);
    assert!(state.tree.contains(node), "nothing is removed locally when the broker cannot be told");

    connect(&mut state);
    receive(&mut state, &[("a/b", "1"), ("a/c", "2")]);
    let node = state.tree.find("a/b").unwrap();
    state.apply(Action::Select(Some(node)));
    state.apply(Action::DeleteRetained { node, recursive: false });
    assert_eq!(published(&mut state), [PublishRequest::delete_retained("a/b")]);
    assert_eq!(state.tree.find("a/b"), None);
    assert_eq!(state.selected, None);

    // The broker echoes the empty message; it must not bring the topic back. A later empty message is real.
    receive(&mut state, &[("a/b", "")]);
    assert_eq!(state.tree.find("a/b"), None);
    receive(&mut state, &[("a/b", "")]);
    assert!(state.tree.find("a/b").is_some());

    state.apply(Action::ClearTree);
    state.apply(Action::DeleteRetained { node, recursive: false });
    assert_eq!(state.take_commands().len(), 0, "a stale node is ignored");
}

#[test]
fn deleting_several_topics_asks_first() {
    let mut state = state();
    connect(&mut state);
    receive(&mut state, &[("a/b", "1"), ("a/c", "2"), ("d", "3")]);
    let a = state.tree.find("a").unwrap();
    state.apply(Action::DeleteRetained { node: a, recursive: true });
    assert_eq!(state.take_commands().len(), 0);
    assert!(matches!(&state.dialog, Dialog::ConfirmDelete { topics, .. } if topics == &["a/b", "a/c"]));

    state.apply(Action::CloseDialog);
    assert!(state.tree.contains(a), "cancelling keeps everything");

    state.apply(Action::DeleteRetained { node: a, recursive: true });
    state.apply(Action::ConfirmDelete);
    assert_eq!(published(&mut state).len(), 2);
    assert_eq!(state.tree.stats().topics, 1);
    assert_eq!(state.dialog, Dialog::None);
    state.apply(Action::ConfirmDelete);
    assert_eq!(state.take_commands().len(), 0, "confirming without a dialog does nothing");

    // A subtree with a single topic needs no confirmation.
    receive(&mut state, &[("x/y", "1")]);
    let x = state.tree.find("x").unwrap();
    state.apply(Action::DeleteRetained { node: x, recursive: true });
    assert_eq!(published(&mut state), [PublishRequest::delete_retained("x/y")]);
}

#[test]
fn the_value_view_decodes_caches_and_follows_overrides() {
    let mut state = state();
    assert!(state.selected_value().is_none());
    receive(&mut state, &[("a/json", r#"{"temp":21.5,"nested":{"n":1}}"#), ("a", "parent only")]);
    select(&mut state, "a/json");
    let view = state.selected_value().unwrap().clone();
    assert_eq!(view.decoded.format, Format::Json);
    assert!(view.text.contains("\"temp\": 21.5"));
    assert_eq!(view.chartable, ["/nested/n", "/temp"]);
    assert!(!view.truncated);
    assert_eq!(state.selected_value(), Some(&view), "a second call reuses the cached view");

    state.apply(Action::SetDecoder { pattern: "a/json".into(), format: Format::Hex });
    assert_eq!(state.format_for("a/json"), Format::Hex);
    assert_eq!(state.selected_value().unwrap().decoded.format, Format::Hex);
    state.apply(Action::SetDecoder { pattern: "a/json".into(), format: Format::Auto });
    assert_eq!(state.config.decoder_overrides.len(), 0);
    assert_eq!(state.selected_value().unwrap().decoded.format, Format::Json);

    receive(&mut state, &[("a/json", "plain now")]);
    assert_eq!(state.selected_value().unwrap().text, "plain now");

    let huge = "é".repeat(MAX_VALUE_TEXT);
    receive(&mut state, &[("a/json", &huge)]);
    let view = state.selected_value().unwrap();
    assert!(view.truncated);
    assert!(view.text.len() <= MAX_VALUE_TEXT);

    state.apply(Action::SetValueMode(ValueMode::Raw));
    assert_eq!(state.value_mode, ValueMode::Raw);
}

#[test]
fn a_level_without_its_own_value_has_no_value_view() {
    let mut state = state();
    receive(&mut state, &[("a/b", "1")]);
    select(&mut state, "a");
    assert!(state.selected_value().is_none());
    assert!(state.selected_diff().is_none());
}

#[test]
fn the_diff_compares_with_the_previous_or_a_chosen_message() {
    let mut state = state();
    receive(&mut state, &[("t", r#"{"v":1}"#)]);
    select(&mut state, "t");
    assert!(state.selected_diff().is_none(), "one message has nothing to compare with");

    receive(&mut state, &[("t", r#"{"v":2}"#), ("t", r#"{"v":3}"#)]);
    let texts = |state: &mut AppState| -> Vec<String> {
        state.selected_diff().unwrap().iter().map(|line| format!("{:?} {}", line.kind, line.text.trim())).collect()
    };
    assert_eq!(texts(&mut state), ["Equal {", "Removed \"v\": 2", "Added \"v\": 3", "Equal }"]);
    assert_eq!(texts(&mut state).len(), 4, "cached");

    let first = state.tree.node(state.selected.unwrap()).history()[0].seq();
    state.apply(Action::CompareWith(Some(first)));
    assert_eq!(state.value_mode, ValueMode::Diff);
    assert_eq!(state.compare_target(), Some(first));
    assert_eq!(texts(&mut state)[1], "Removed \"v\": 1");

    state.apply(Action::CompareWith(Some(u64::MAX)));
    assert!(state.selected_diff().is_none(), "an entry that left the history cannot be compared");
    state.apply(Action::CompareWith(None));
    assert_eq!(texts(&mut state)[1], "Removed \"v\": 2");

    // Selecting another topic forgets the comparison target.
    state.apply(Action::CompareWith(Some(first)));
    state.apply(Action::Select(state.selected));
    assert_eq!(state.compare_target(), None);
}

#[test]
fn charts_backfill_from_history_and_follow_live_values() {
    let mut state = state();
    receive(&mut state, &[("env", r#"{"t":1}"#), ("env", r#"{"t":2}"#), ("plain", "5")]);
    state.apply(Action::AddChart { topic: "env".into(), pointer: "/t".into() });
    state.apply(Action::AddChart { topic: "env".into(), pointer: "/t".into() });
    state.apply(Action::AddChart { topic: "plain".into(), pointer: String::new() });
    state.apply(Action::AddChart { topic: "not/seen/yet".into(), pointer: String::new() });
    assert_eq!(state.charts.len(), 3, "the same series is not added twice");
    assert_eq!(state.charts[0].points.len(), 2);
    assert_eq!(state.charts[1].latest(), Some(5.0));

    receive(&mut state, &[("env", r#"{"t":3}"#), ("not/seen/yet", "7"), ("other", "9")]);
    assert_eq!(state.charts[0].latest(), Some(3.0));
    assert_eq!(state.charts[2].latest(), Some(7.0));

    state.apply(Action::ClearTree);
    assert!(state.charts.iter().all(|chart| chart.points.is_empty()));
    state.apply(Action::RemoveChart(0));
    state.apply(Action::RemoveChart(10));
    assert_eq!(state.charts.len(), 2);
}

#[test]
fn recording_writes_received_messages_to_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut state = state();
    assert!(state.recording().is_none());
    state.apply(Action::StopRecording);
    state.apply(Action::StartRecording(dir.path().to_path_buf()));
    assert!(notices(&state)[0].starts_with("!Recording could not be started"));

    state.apply(Action::StartRecording(path.clone()));
    receive(&mut state, &[("a", "1"), ("b", "2")]);
    assert_eq!(state.recording(), Some((path.as_path(), 2)));
    state.apply(Action::StopRecording);
    assert!(notices(&state).last().unwrap().starts_with("Recorded 2 messages"));
    receive(&mut state, &[("c", "3")]);
    assert_eq!(mqtt_viewer_core::record::read_recording(&path).unwrap().len(), 2);
}

#[test]
fn messages_held_by_pause_are_recorded_when_released() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut state = state();
    state.apply(Action::StartRecording(path));
    state.apply(Action::TogglePause);
    receive(&mut state, &[("a", "1")]);
    assert_eq!(state.recording().unwrap().1, 0);
    state.apply(Action::TogglePause);
    assert_eq!(state.recording().unwrap().1, 1);
}

#[test]
fn exports_cover_value_history_and_subtree() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = state();
    let target = |name: &str| dir.path().join(name);
    state.apply(Action::Export { scope: ExportScope::Value, format: ExportFormat::Json, path: target("none.json") });
    assert_eq!(notices(&state), ["!Select a topic to export"]);

    receive(&mut state, &[("a/b", "1"), ("a/b", "2"), ("a/c", "3")]);
    select(&mut state, "a/b");
    let lines = |path: PathBuf| std::fs::read_to_string(path).unwrap().lines().count();
    state.apply(Action::Export { scope: ExportScope::Value, format: ExportFormat::Csv, path: target("value.csv") });
    assert_eq!(lines(target("value.csv")), 2, "header plus the latest value");
    state.apply(Action::Export { scope: ExportScope::History, format: ExportFormat::Csv, path: target("history.csv") });
    assert_eq!(lines(target("history.csv")), 3);

    select(&mut state, "a");
    state.apply(Action::Export { scope: ExportScope::Subtree, format: ExportFormat::Json, path: target("tree.json") });
    let exported: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(target("tree.json")).unwrap()).unwrap();
    let topics: Vec<&str> = exported.as_array().unwrap().iter().map(|row| row["topic"].as_str().unwrap()).collect();
    assert_eq!(topics, ["a/b", "a/c"]);
    assert_eq!(exported[0]["payload"], "2", "the subtree export holds latest values");

    state.notices.clear();
    state.apply(Action::Export { scope: ExportScope::Value, format: ExportFormat::Json, path: target("empty.json") });
    assert_eq!(notices(&state), ["!There is nothing to export for this topic"]);
    state.apply(Action::Export {
        scope: ExportScope::Subtree,
        format: ExportFormat::Json,
        path: dir.path().to_path_buf(),
    });
    assert!(notices(&state)[1].starts_with("!Export to"), "{:?}", notices(&state));
}

#[test]
fn replay_needs_a_recording_and_a_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut state = state();
    state.apply(Action::LoadRecording(path.clone()));
    assert!(notices(&state)[0].starts_with("!Recording could not be read"));
    std::fs::write(&path, "").unwrap();
    state.apply(Action::LoadRecording(path.clone()));
    assert!(notices(&state)[1].ends_with("contains no messages"));
    state.apply(Action::StartReplay);
    assert_eq!(state.take_commands().len(), 0, "nothing to start without the dialog");

    state.apply(Action::StartRecording(path.clone()));
    receive(&mut state, &[("home/a", "1"), ("home/b", "2")]);
    state.apply(Action::StopRecording);
    state.notices.clear();

    state.apply(Action::LoadRecording(path));
    let Dialog::Replay(dialog) = &mut state.dialog else { panic!("expected the replay dialog") };
    assert_eq!(dialog.messages.len(), 2);
    dialog.options.prefix_from = "home/".into();
    dialog.options.prefix_to = "replay/".into();
    state.apply(Action::StartReplay);
    assert_eq!(notices(&state), ["!Connect to a broker before replaying"]);
    assert!(matches!(state.dialog, Dialog::Replay(_)), "the dialog stays open so the setup is not lost");

    state.handle_event(Event::State(ConnectionState::Connected));
    state.apply(Action::StartReplay);
    let commands = state.take_commands();
    let [Command::Replay(steps)] = &commands[..] else { panic!("expected a replay command, got {commands:?}") };
    assert_eq!(steps.iter().map(|step| step.request.topic.as_str()).collect::<Vec<_>>(), ["replay/a", "replay/b"]);
    assert_eq!(state.replay, Some((0, 2)));
    assert_eq!(state.dialog, Dialog::None);

    state.handle_event(Event::ReplayProgress { sent: 1, total: 2 });
    assert_eq!(state.replay, Some((1, 2)));
    state.apply(Action::StopReplay);
    assert!(matches!(state.take_commands()[..], [Command::StopReplay]));
    state.notices.clear();
    state.handle_event(Event::ReplayFinished { sent: 1, total: 2 });
    assert_eq!(state.replay, None);
    assert_eq!(notices(&state), ["Replay finished: 1 of 2 messages published"]);
    state.handle_event(Event::ReplayFinished { sent: 0, total: 0 });
    assert_eq!(notices(&state).len(), 1, "a finish without a running replay is not announced");
}

#[test]
fn settings_are_applied_and_saved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut state = AppState::new(AppConfig::default(), Some(path.clone()), None);
    for index in 0..10 {
        receive(&mut state, &[("t", &index.to_string())]);
    }
    state.apply(Action::ApplySettings);
    assert!(!path.exists(), "applying without the dialog does nothing");

    state.apply(Action::OpenSettings);
    let Dialog::Settings(draft) = &mut state.dialog else { panic!("expected the settings dialog") };
    draft.history_cap = 3;
    draft.tree_order = TreeOrder::Topics;
    draft.memory_budget_mb = 16;
    state.apply(Action::ApplySettings);
    assert_eq!(state.dialog, Dialog::None);
    assert_eq!(state.tree.node(state.tree.find("t").unwrap()).history().len(), 3);
    assert_eq!(state.tree.order(), TreeOrder::Topics);
    assert_eq!(state.tree.limits().total_bytes, 16 * 1024 * 1024);
    assert_eq!(AppConfig::load(&path).unwrap().settings.history_cap, 3);

    // Cancelling discards the draft.
    state.apply(Action::OpenSettings);
    if let Dialog::Settings(draft) = &mut state.dialog {
        draft.history_cap = 999;
    }
    state.apply(Action::CloseDialog);
    assert_eq!(state.config.settings.history_cap, 3);
}

fn enable_persistence(state: &mut AppState) {
    state.apply(Action::OpenSettings);
    let Dialog::Settings(draft) = &mut state.dialog else { panic!("expected the settings dialog") };
    draft.persist_history = true;
    state.apply(Action::ApplySettings);
}

#[test]
fn history_is_stored_and_restored_when_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("history.sqlite");
    let mut state = AppState::new(AppConfig::default(), Some(dir.path().join("config.toml")), Some(database.clone()));
    assert!(!state.is_persisting());
    connect(&mut state);
    receive(&mut state, &[("before", "not stored")]);
    assert!(!database.exists(), "nothing touches the disk while the setting is off");

    enable_persistence(&mut state);
    assert!(state.is_persisting());
    receive(&mut state, &[("a", "1"), ("a", "2"), ("b", "3")]);

    // Reconnecting with the same profile brings the stored messages back, in order.
    state.apply(Action::OpenConnectDialog);
    state.apply(Action::Connect);
    assert_eq!(state.tree.stats().topics, 2);
    assert_eq!(state.tree.node(state.tree.find("a").unwrap()).history().len(), 2);
    assert_eq!(notices(&state).last().unwrap(), "Restored 3 stored messages");

    // Restored messages are not written a second time.
    drop(state);
    assert_eq!(Store::open(&database).unwrap().count().unwrap(), 3);

    // A fresh session with the saved setting restores on connect, and a different profile sees nothing.
    let mut state = AppState::load_from(Some(dir.path().join("config.toml")), Some(database.clone()));
    assert!(state.is_persisting());
    state.apply(Action::OpenConnectDialog);
    state.apply(Action::Connect);
    assert_eq!(state.tree.stats().messages, 3);
    state.apply(Action::NewProfile);
    state.apply(Action::Connect);
    assert_eq!(state.tree.stats().messages, 0);

    state.apply(Action::ClearStoredHistory);
    assert_eq!(notices(&state).last().unwrap(), "Stored history cleared");
    drop(state);
    assert_eq!(Store::open(&database).unwrap().count().unwrap(), 0);
}

#[test]
fn turning_persistence_off_stops_writing() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("history.sqlite");
    let mut state = AppState::new(AppConfig::default(), None, Some(database.clone()));
    connect(&mut state);
    enable_persistence(&mut state);
    receive(&mut state, &[("a", "1")]);
    state.apply(Action::OpenSettings);
    if let Dialog::Settings(draft) = &mut state.dialog {
        draft.persist_history = false;
    }
    state.apply(Action::ApplySettings);
    assert!(!state.is_persisting());
    receive(&mut state, &[("a", "2")]);
    assert_eq!(Store::open(&database).unwrap().count().unwrap(), 1);

    // The leftover database can still be cleared.
    state.apply(Action::ClearStoredHistory);
    assert_eq!(notices(&state).last().unwrap(), "Stored history cleared (1 messages)");
    assert_eq!(Store::open(&database).unwrap().count().unwrap(), 0);
}

#[test]
fn persistence_problems_are_reported() {
    let mut without_directory = state();
    enable_persistence(&mut without_directory);
    assert_eq!(notices(&without_directory), ["!History cannot be stored: no data directory is available"]);
    assert!(!without_directory.is_persisting());
    without_directory.apply(Action::ClearStoredHistory);
    assert_eq!(notices(&without_directory)[1], "There is no stored history");

    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("file");
    std::fs::write(&blocker, "x").unwrap();
    let mut blocked = AppState::new(AppConfig::default(), None, Some(blocker.join("history.sqlite")));
    enable_persistence(&mut blocked);
    assert!(notices(&blocked)[0].starts_with("!history database"), "{:?}", notices(&blocked));
    assert!(!blocked.is_persisting());

    let garbage = dir.path().join("garbage.sqlite");
    std::fs::write(&garbage, "definitely not a database file, just some text").unwrap();
    let mut corrupt = AppState::new(AppConfig::default(), None, Some(garbage));
    corrupt.apply(Action::ClearStoredHistory);
    assert!(notices(&corrupt)[0].starts_with('!'), "{:?}", notices(&corrupt));
}

#[test]
fn protobuf_schemas_and_bindings_are_managed() {
    let dir = tempfile::tempdir().unwrap();
    let proto = dir.path().join("t.proto");
    std::fs::write(&proto, "syntax = \"proto3\";\npackage acme;\nmessage Reading { double value = 1; }\n").unwrap();
    let mut state = AppState::new(AppConfig::default(), Some(dir.path().join("config.toml")), None);

    state.apply(Action::AddProtoFile(proto.clone()));
    state.apply(Action::AddProtoFile(proto));
    assert_eq!(state.config.proto_files.len(), 1);
    assert_eq!(state.decoder.registry().message_types(), ["acme.Reading"]);

    state.apply(Action::AddProtoBinding(ProtoBinding { pattern: "a/#/b".into(), message_type: "acme.Reading".into() }));
    state.apply(Action::AddProtoBinding(ProtoBinding { pattern: "sensors/#".into(), message_type: " ".into() }));
    assert_eq!(notices(&state).len(), 2);
    assert_eq!(state.config.proto_bindings.len(), 0);
    state.apply(Action::AddProtoBinding(ProtoBinding {
        pattern: "sensors/#".into(),
        message_type: "acme.Other".into(),
    }));
    state.apply(Action::AddProtoBinding(ProtoBinding {
        pattern: "sensors/#".into(),
        message_type: "acme.Reading".into(),
    }));
    assert_eq!(state.config.proto_bindings.len(), 1, "one binding per pattern");

    // field 1, fixed64, 21.5
    let payload = [&[0x09_u8][..], &21.5_f64.to_le_bytes()].concat();
    state.handle_event(Event::Messages(vec![Message::new("sensors/a", payload)]));
    select(&mut state, "sensors/a");
    let view = state.selected_value().unwrap();
    assert_eq!(view.decoded.format, Format::Protobuf);
    assert_eq!(view.chartable, ["/value"]);

    state.apply(Action::RemoveProtoBinding(0));
    state.apply(Action::RemoveProtoBinding(0));
    assert_ne!(state.selected_value().unwrap().decoded.format, Format::Protobuf);
    state.apply(Action::RemoveProtoFile(0));
    state.apply(Action::RemoveProtoFile(0));
    assert_eq!(state.decoder.registry().message_types().len(), 0);

    state.notices.clear();
    state.apply(Action::AddProtoFile(dir.path().join("missing.proto")));
    assert!(notices(&state)[0].starts_with("!Protobuf schema"), "{:?}", notices(&state));

    // Schemas listed in the configuration are loaded at startup.
    let reloaded = AppState::load_from(Some(dir.path().join("config.toml")), None);
    assert_eq!(notices(&reloaded).len(), 1, "the missing file is reported again");
}

#[test]
fn notices_expire_and_can_be_dismissed() {
    let mut state = state();
    state.info("one");
    state.error("two");
    state.apply(Action::DismissNotice(0));
    state.apply(Action::DismissNotice(7));
    assert_eq!(notices(&state), ["!two"]);
    state.tick(Instant::now());
    assert_eq!(state.notices.len(), 1);
    state.tick(Instant::now() + NOTICE_LIFETIME + Duration::from_secs(1));
    assert_eq!(state.notices.len(), 0);
}

#[test]
fn rate_and_activity_are_measured() {
    let mut state = state();
    let start = Instant::now();
    state.tick(start);
    let batch: Vec<Message> = (0..100).map(|index| message("t", &index.to_string(), 0)).collect();
    state.handle_event(Event::Messages(batch));
    state.tick(start + Duration::from_secs(2));
    assert!((state.message_rate() - 50.0).abs() < 1e-9, "{}", state.message_rate());

    let node = state.tree.find("t").unwrap();
    let at = state.tree.node(node).last_activity().unwrap();
    let age = state.activity_age(node, at + jiff::SignedDuration::from_millis(1500)).unwrap();
    assert!((age - 1.5).abs() < 1e-9);
    assert!(state.activity_age(TopicTree::ROOT, Timestamp::now()).is_some());
}

#[test]
fn a_broken_config_file_is_backed_up_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "profiles = \"oops\"").unwrap();
    let mut state = AppState::load_from(Some(path.clone()), None);
    let shown = notices(&state);
    assert!(shown[0].contains("is invalid") && shown[0].contains("config.toml.broken"), "{shown:?}");
    assert_eq!(std::fs::read_to_string(dir.path().join("config.toml.broken")).unwrap(), "profiles = \"oops\"");

    // Saving now is safe: the original content survives in the backup.
    state.apply(Action::SetOrder(TreeOrder::Messages));
    assert_eq!(AppConfig::load(&path).unwrap().settings.tree_order, TreeOrder::Messages);

    assert_eq!(notices(&AppState::load_from(None, None)).len(), 0);
    assert_eq!(notices(&AppState::load_from(Some(dir.path().join("fresh.toml")), None)).len(), 0);
}

#[test]
fn save_failures_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(AppConfig::default(), Some(dir.path().to_path_buf()), None);
    state.apply(Action::SetOrder(TreeOrder::Topics));
    assert!(notices(&state)[0].starts_with("!config file"), "{:?}", notices(&state));
}

#[test]
fn the_sample_session_is_consistent() {
    let mut state = populated();
    assert_eq!(state.tree.stats().topics, 12);
    assert_eq!(state.connection, ConnectionState::Connected);
    select(&mut state, "home/kitchen/climate");
    assert_eq!(state.selected_value().unwrap().chartable, ["/humidity", "/ok", "/temperature"]);
}

#[test]
fn the_publish_topic_follows_the_selection_until_edited() {
    let mut state = state();
    receive(&mut state, &[("a/b", "1"), ("a/c", "2"), ("d", "3")]);
    select(&mut state, "a/b");
    assert_eq!(state.publish.topic, "a/b");
    select(&mut state, "a/c");
    assert_eq!(state.publish.topic, "a/c");
    state.apply(Action::Navigate(Navigate::Down));
    assert_eq!(state.publish.topic, "d", "keyboard selection counts too");

    state.publish.topic = "my/own/topic".into();
    select(&mut state, "a/b");
    assert_eq!(state.publish.topic, "my/own/topic", "a typed topic is never overwritten");
    state.apply(Action::Select(None));
    assert_eq!(state.publish.topic, "my/own/topic");
}
