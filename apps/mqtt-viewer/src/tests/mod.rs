mod app_test;
mod broker;
mod chart_test;
mod format_test;
mod forms_test;
mod highlight_test;
mod icon_test;
mod screenshot_test;
mod state_test;
mod ui_test;

use jiff::Timestamp;
use mqtt_viewer_core::config::AppConfig;
use mqtt_viewer_core::connection::{ConnectionState, Event};
use mqtt_viewer_core::message::Message;

use crate::state::{Action, AppState};

/// A state that persists nothing.
pub fn state() -> AppState {
    let mut state = AppState::new(AppConfig::default(), None, None);
    // Tests read the rows right after delivering messages and expect them to be current.
    state.tree.set_refresh_interval(std::time::Duration::ZERO);
    state
}

/// A message with a fixed timestamp `seconds` after an arbitrary start.
pub fn message(topic: &str, payload: &str, seconds: i64) -> Message {
    Message {
        received_at: Timestamp::from_second(1_800_000_000 + seconds).unwrap(),
        ..Message::new(topic, payload.to_owned())
    }
}

/// Marks `state` as connected with the default profile, without any queued commands or notices.
pub fn connect(state: &mut AppState) {
    state.apply(Action::OpenConnectDialog);
    state.apply(Action::Connect);
    state.take_commands();
    state.handle_event(Event::State(ConnectionState::Connected));
    state.notices.clear();
}

/// Delivers messages as the connection worker would.
pub fn receive(state: &mut AppState, messages: &[(&str, &str)]) {
    let batch = messages.iter().zip(0..).map(|((topic, payload), second)| message(topic, payload, second)).collect();
    state.handle_event(Event::Messages(batch));
}

/// The texts of all notices, errors prefixed with `!`.
pub fn notices(state: &AppState) -> Vec<String> {
    state.notices.iter().map(|notice| format!("{}{}", if notice.is_error { "!" } else { "" }, notice.text)).collect()
}

/// A state that looks like a live session against a home-automation broker.
pub fn populated() -> AppState {
    let mut state = state();
    connect(&mut state);
    let mut batch = Vec::new();
    for (index, room) in ["kitchen", "livingroom", "bedroom", "garage"].iter().enumerate() {
        for second in 0..30_i64 {
            let offset = i64::try_from(index).unwrap();
            let temperature = 20.0 + f64::from(i32::try_from((second * 7 + offset * 3) % 10).unwrap()) / 4.0;
            batch.push(message(
                &format!("home/{room}/climate"),
                &format!(r#"{{"temperature":{temperature},"humidity":{},"unit":"C","ok":true}}"#, 40 + second % 5),
                second,
            ));
        }
        batch.push(message(&format!("home/{room}/light"), if index % 2 == 0 { "ON" } else { "OFF" }, 30));
    }
    batch.push(message("home/garage/door", "closed", 31));
    batch.push(message("devices/pump-1/raw", "\u{0}\u{1}\u{2}\u{3}binary", 32));
    batch.push(message("$SYS/broker/uptime", "86400 seconds", 33));
    batch.push(message("$SYS/broker/clients/connected", "12", 33));
    state.handle_event(Event::Messages(batch));
    state
}
