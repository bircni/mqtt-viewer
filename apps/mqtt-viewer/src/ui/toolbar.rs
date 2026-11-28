//! The top toolbar and the status bar.

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::{self, Button, CornerRadius, Frame, Margin, PopupCloseBehavior, RichText, Stroke, TextEdit};
use mqtt_viewer_core::config::{ConnectionProfile, Subscription, TreeOrder};
use mqtt_viewer_core::connection::ConnectionState;
use mqtt_viewer_core::message::QoS;

use super::fonts::icon;
use super::theme::{self, Palette};
use crate::format;
use crate::state::{Action, AppState};

pub fn top(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.horizontal(|ui| {
        brand(ui, &palette);
        ui.add_space(6.0);
        connection(ui, state, actions, &palette);
        ui.add_space(4.0);
        search(ui, state, actions, &palette);
        ui.add_space(4.0);
        if state.connection.is_active() {
            subscriptions(ui, state, actions, &palette);
        }

        let pause = if state.paused { format!("Resume ({})", state.held_count()) } else { "Pause".to_owned() };
        let (glyph, color) = if state.paused { (icon::PLAY, palette.warn) } else { (icon::PAUSE, palette.text_weak) };
        let button = Button::selectable(state.paused, (theme::icon(glyph, color), pause.as_str()));
        if ui.add(button).accessible_name(&pause).on_hover_text("Hold back incoming messages (Space)").clicked() {
            actions.push(Action::TogglePause);
        }
        order(ui, state, actions);
        if theme::icon_only(ui, icon::BROOM, "Clear").on_hover_text("Remove all topics from the view").clicked() {
            actions.push(Action::ClearTree);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_only(ui, icon::GEAR_SIX, "Settings").clicked() {
                actions.push(Action::OpenSettings);
            }
            recording(ui, state, actions);
        });
    });
}

fn brand(ui: &mut egui::Ui, palette: &Palette) {
    Frame::new().fill(palette.accent).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(5)).show(
        ui,
        |ui| {
            ui.label(RichText::new(icon::TREE_STRUCTURE).size(16.0).color(palette.on_accent));
        },
    );
    ui.label(theme::strong("MQTT Viewer", 14.5).color(palette.text_strong));
}

fn connection(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>, palette: &Palette) {
    let (color, label) = match &state.connection {
        ConnectionState::Disconnected => (palette.error, "Disconnected".to_owned()),
        ConnectionState::Connecting => (palette.warn, "Connecting…".to_owned()),
        ConnectionState::Connected => (palette.ok, "Connected".to_owned()),
        ConnectionState::Reconnecting { attempt, .. } => (palette.warn, format!("Reconnecting (attempt {attempt})…")),
    };
    let active = state.profile.as_ref().filter(|_| state.connection.is_active());
    let response = Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin { left: 10, right: 4, top: 3, bottom: 3 })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            theme::dot(ui, color, 3.5);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let (name, detail) = active
                    .map_or_else(|| ("Offline".to_owned(), "No broker".to_owned()), |p| (p.name.clone(), p.url()));
                ui.label(theme::strong(name, 13.0).color(palette.text_strong));
                ui.label(RichText::new(detail).size(11.0).color(palette.text_weak));
            });
            ui.add_space(6.0);
            if active.is_some() {
                if theme::icon_only(ui, icon::PLUGS, "Disconnect").clicked() {
                    actions.push(Action::Disconnect);
                }
            } else if theme::add_filled(
                ui,
                theme::primary_button(ui, Some(icon::PLUGS_CONNECTED), "Connect…"),
                "Connect…",
            )
            .clicked()
            {
                actions.push(Action::OpenConnectDialog);
            }
        })
        .response;
    response.on_hover_text(label);
}

fn search(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>, palette: &Palette) {
    let focused = ui.memory(egui::Memory::focused).is_some_and(|id| id == ui.make_persistent_id("search"));
    Frame::new()
        .fill(palette.sunken)
        .stroke(Stroke::new(1.0, if focused { palette.accent } else { palette.border }))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(theme::icon(icon::MAGNIFYING_GLASS, palette.text_weak));
            let search = ui
                .add(
                    TextEdit::singleline(&mut state.search_text)
                        .id(ui.make_persistent_id("search"))
                        .hint_text("Search topics…")
                        .frame(egui::Frame::NONE)
                        .margin(Margin::ZERO)
                        .align(egui::Align2::LEFT_CENTER)
                        .min_size(egui::vec2(0.0, 26.0))
                        .desired_width(220.0),
                )
                .accessible_name("Search topics");
            if std::mem::take(&mut state.focus_search) {
                search.request_focus();
            }
            let clear = !state.search_text.is_empty() && theme::icon_only(ui, icon::X, "Clear search").clicked();
            if clear {
                state.search_text.clear();
            }
            let payload = ui
                .toggle_value(&mut state.search_payload, RichText::new("values").size(12.0))
                .on_hover_text("Also search the latest payloads");
            if search.changed() || payload.changed() || clear {
                actions.push(Action::SearchChanged);
            }
        });
}

/// The running connection's subscriptions, in a popup that stays open while it is edited.
fn subscriptions(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>, palette: &Palette) {
    let active = state.profile.as_ref().map_or(&[][..], |profile| profile.subscriptions.as_slice());
    let label = format::plural(active.len() as u64, "subscription");
    let button = Button::new((theme::icon(icon::HASH, palette.text_weak), label.as_str()));
    let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    let (response, _) = MenuButton::from_button(button).config(config).ui(ui, |ui| {
        ui.set_min_width(340.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(theme::strong("Subscriptions", 13.0).color(palette.text_strong));
        if active.is_empty() {
            ui.weak("Nothing is subscribed; no messages arrive.");
        }
        for subscription in active {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&subscription.filter).monospace().color(palette.text));
                theme::badge(ui, format!("QoS {}", subscription.qos.level()), palette.text_weak);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let name = format!("Unsubscribe {}", subscription.filter);
                    if theme::icon_only(ui, icon::X, &name).clicked() {
                        actions.push(Action::Unsubscribe(subscription.filter.clone()));
                    }
                });
            });
        }
        ui.separator();
        let id = ui.make_persistent_id("new-subscription");
        let mut draft =
            ui.data(|data| data.get_temp::<(String, QoS)>(id)).unwrap_or_else(|| (String::new(), QoS::AtMostOnce));
        ui.horizontal(|ui| {
            let field = ui
                .add(
                    TextEdit::singleline(&mut draft.0)
                        .hint_text("topic/filter/#")
                        .font(egui::TextStyle::Monospace)
                        .margin(egui::vec2(8.0, 5.0))
                        .desired_width(150.0),
                )
                .accessible_name("New subscription");
            let levels = QoS::ALL.map(|qos| (qos, ["QoS 0", "QoS 1", "QoS 2"][usize::from(qos.level())]));
            theme::segmented_value(ui, &mut draft.1, &levels);
            let submit = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if (theme::add_filled(ui, theme::primary_button(ui, None, "Subscribe"), "Subscribe").clicked() || submit)
                && !draft.0.trim().is_empty()
            {
                actions.push(Action::Subscribe(Subscription { filter: std::mem::take(&mut draft.0), qos: draft.1 }));
            }
        });
        ui.weak("Changes last for this connection; the saved profile keeps its list.");
        ui.data_mut(|data| data.insert_temp(id, draft));
    });
    response.accessible_name("Subscriptions").on_hover_text("Subscriptions of this connection");
}

fn order(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    let order = state.tree.order();
    egui::ComboBox::from_id_salt("order")
        .selected_text(order_label(order))
        .width(130.0)
        .show_ui(ui, |ui| {
            for option in [TreeOrder::Alphabetical, TreeOrder::Messages, TreeOrder::Topics] {
                if ui.selectable_label(order == option, order_label(option)).clicked() {
                    actions.push(Action::SetOrder(option));
                }
            }
        })
        .response
        .accessible_name("Topic order")
        .on_hover_text("Topic order");
}

/// Recording and replay. Laid out right to left, so the last button added is the leftmost.
fn recording(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    if let Some((sent, total)) = state.replay {
        let label = format!("Stop replay {sent}/{total}");
        if theme::icon_button(ui, icon::STOP, &label).on_hover_text("Stop the replay").clicked() {
            actions.push(Action::StopReplay);
        }
    } else if theme::icon_only(ui, icon::PLAY_CIRCLE, "Replay…")
        .on_hover_text("Publish a recording to the connected broker")
        .clicked()
        && let Some(path) = state.files.open("Recording", &["jsonl"])
    {
        actions.push(Action::LoadRecording(path));
    }
    if let Some((path, count)) = state.recording() {
        let label = format!("Stop recording ({})", format::count(count));
        if theme::add_filled(ui, theme::danger_button(ui, Some(icon::STOP), &label), &label)
            .on_hover_text(format!("Recording to {}", path.display()))
            .clicked()
        {
            actions.push(Action::StopRecording);
        }
    } else if theme::icon_only(ui, icon::RECORD, "Record").on_hover_text("Record received messages to a file").clicked()
        && let Some(path) = state.files.save("recording.jsonl", "Recording", &["jsonl"])
    {
        actions.push(Action::StartRecording(path));
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
    let palette = Palette::of(ui);
    let stats = state.tree.stats();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let text = match &state.connection {
            ConnectionState::Disconnected => "Not connected".to_owned(),
            ConnectionState::Connecting => "Connecting…".to_owned(),
            ConnectionState::Connected => state.profile.as_ref().map_or_else(String::new, ConnectionProfile::url),
            ConnectionState::Reconnecting { attempt, error } => format!("Reconnecting, attempt {attempt}: {error}"),
        };
        ui.label(RichText::new(text).small().color(palette.text_weak));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let item = |ui: &mut egui::Ui, glyph: &str, text: String| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new(text).small().color(palette.text));
                    ui.label(RichText::new(glyph).size(12.0).color(palette.text_weak));
                })
                .response
            };
            item(ui, icon::DATABASE, format::bytes(stats.stored_bytes)).on_hover_text("Memory used by message history");
            item(ui, icon::CHART_LINE, format::rate(state.message_rate()));
            item(ui, icon::PAPER_PLANE_TILT, format::plural(stats.messages, "message"));
            item(ui, icon::TREE_STRUCTURE, format::plural(stats.topics as u64, "topic"));
            if state.is_persisting() {
                item(ui, icon::FLOPPY_DISK, "history on disk".to_owned())
                    .on_hover_text("Received messages are stored in the local database");
            }
        });
    });
}
