//! The top toolbar and the status bar.

use eframe::egui::{self, RichText, TextEdit};
use mqtt_viewer_core::config::{ConnectionProfile, TreeOrder};
use mqtt_viewer_core::connection::ConnectionState;

use super::theme::Palette;
use crate::format;
use crate::state::{Action, AppState};

pub fn top(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (color, label) = match &state.connection {
            ConnectionState::Disconnected => (palette.error, "Disconnected".to_owned()),
            ConnectionState::Connecting => (palette.warn, "Connecting…".to_owned()),
            ConnectionState::Connected => (palette.ok, "Connected".to_owned()),
            ConnectionState::Reconnecting { attempt, .. } => {
                (palette.warn, format!("Reconnecting (attempt {attempt})…"))
            }
        };
        let (dot, response) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        ui.painter().circle_filled(dot.center(), 5.0, color);
        response.on_hover_text(&label);
        if state.connection.is_active() {
            if let Some(profile) = &state.profile {
                ui.label(RichText::new(&profile.name).strong()).on_hover_text(profile.url());
            }
            if ui.button("Disconnect").clicked() {
                actions.push(Action::Disconnect);
            }
        } else if ui.button("Connect…").clicked() {
            actions.push(Action::OpenConnectDialog);
        }
        ui.separator();

        let search = ui
            .add(TextEdit::singleline(&mut state.search_text).hint_text("Search topics…").desired_width(220.0))
            .accessible_name("Search topics");
        let payload = ui.checkbox(&mut state.search_payload, "values").on_hover_text("Also search the latest payloads");
        if search.changed() || payload.changed() {
            actions.push(Action::SearchChanged);
        }
        if !state.search_text.is_empty() && ui.small_button("×").on_hover_text("Clear search").clicked() {
            state.search_text.clear();
            actions.push(Action::SearchChanged);
        }
        ui.separator();

        let pause = if state.paused { format!("Resume ({})", state.held_count()) } else { "Pause".to_owned() };
        if ui.selectable_label(state.paused, pause).on_hover_text("Hold back incoming messages (Space)").clicked() {
            actions.push(Action::TogglePause);
        }
        let order = state.tree.order();
        egui::ComboBox::from_id_salt("order")
            .selected_text(order_label(order))
            .width(120.0)
            .show_ui(ui, |ui| {
                for option in [TreeOrder::Alphabetical, TreeOrder::Messages, TreeOrder::Topics] {
                    if ui.selectable_label(order == option, order_label(option)).clicked() {
                        actions.push(Action::SetOrder(option));
                    }
                }
            })
            .response
            .accessible_name("Topic order");
        if ui.button("Clear").on_hover_text("Remove all topics from the view").clicked() {
            actions.push(Action::ClearTree);
        }
        ui.separator();
        recording(ui, state, actions, &palette);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Settings").clicked() {
                actions.push(Action::OpenSettings);
            }
        });
    });
    ui.add_space(4.0);
}

fn recording(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>, palette: &Palette) {
    if let Some((path, count)) = state.recording() {
        let label = RichText::new(format!("Stop recording ({})", format::count(count))).color(palette.error);
        if ui.button(label).on_hover_text(format!("Recording to {}", path.display())).clicked() {
            actions.push(Action::StopRecording);
        }
    } else if ui.button("Record").on_hover_text("Record received messages to a file").clicked()
        && let Some(path) =
            rfd::FileDialog::new().set_file_name("recording.jsonl").add_filter("Recording", &["jsonl"]).save_file()
    {
        actions.push(Action::StartRecording(path));
    }
    if let Some((sent, total)) = state.replay {
        if ui.button(format!("Stop replay {sent}/{total}")).on_hover_text("Stop the replay").clicked() {
            actions.push(Action::StopReplay);
        }
    } else if ui.button("Replay…").on_hover_text("Publish a recording to the connected broker").clicked()
        && let Some(path) = rfd::FileDialog::new().add_filter("Recording", &["jsonl"]).pick_file()
    {
        actions.push(Action::LoadRecording(path));
    }
}

const fn order_label(order: TreeOrder) -> &'static str {
    match order {
        TreeOrder::Alphabetical => "A–Z",
        TreeOrder::Messages => "Most messages",
        TreeOrder::Topics => "Most topics",
    }
}

pub fn status(ui: &mut egui::Ui, state: &AppState) {
    let stats = state.tree.stats();
    ui.horizontal(|ui| {
        let text = match &state.connection {
            ConnectionState::Disconnected => "Not connected".to_owned(),
            ConnectionState::Connecting => "Connecting…".to_owned(),
            ConnectionState::Connected => state.profile.as_ref().map_or_else(String::new, ConnectionProfile::url),
            ConnectionState::Reconnecting { attempt, error } => format!("Reconnecting, attempt {attempt}: {error}"),
        };
        ui.label(text);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format::bytes(stats.stored_bytes)).on_hover_text("Memory used by message history");
            ui.separator();
            ui.label(format::rate(state.message_rate()));
            ui.separator();
            ui.label(format::plural(stats.messages, "message"));
            ui.separator();
            ui.label(format::plural(stats.topics as u64, "topic"));
            if state.is_persisting() {
                ui.separator();
                ui.label("history on disk").on_hover_text("Received messages are stored in the local database");
            }
        });
    });
}
