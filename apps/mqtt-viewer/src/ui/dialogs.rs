//! Modal dialogs: connection profiles (in [`super::connect`]), settings, delete confirmation and replay.

use eframe::egui::{self, Align, DragValue, Frame, Layout, Margin, Modal, RichText, ScrollArea, Stroke, TextEdit};
use mqtt_viewer_core::config::{ConnectionProfile, ProtoBinding, Settings, Theme, TimestampFormat, TreeOrder};
use mqtt_viewer_core::record;

use super::connect;
use super::fonts::icon;
use super::theme::{self, Palette};
use crate::state::{Action, AppState, Dialog, ReplayDialog};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SettingsTab {
    #[default]
    General,
    Storage,
    Protobuf,
}

/// Width of the label column in the settings dialog.
const LABEL_WIDTH: f32 = 150.0;

pub fn show(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    // The dialog is taken out while it is drawn so its body can read the rest of the state.
    let mut dialog = std::mem::take(&mut state.dialog);
    let frame = theme::dialog_frame(ctx);
    let backdrop = egui::Color32::from_black_alpha(150);
    let modal = |id: &str| Modal::new(egui::Id::unique(id)).frame(frame).backdrop_color(backdrop);
    match &mut dialog {
        Dialog::None => {}
        Dialog::Connect(connect) => {
            modal("connect").show(ctx, |ui| connect::show(ui, state, connect, actions));
        }
        Dialog::Settings(draft) => {
            modal("settings").show(ctx, |ui| settings_dialog(ui, state, draft, actions));
        }
        Dialog::ConfirmDelete { topics, .. } => {
            modal("confirm-delete").show(ctx, |ui| confirm_delete(ui, topics, actions));
        }
        Dialog::Replay(replay) => {
            modal("replay").show(ctx, |ui| replay_dialog(ui, state, replay, actions));
        }
    }
    state.dialog = dialog;
}

/// The top of a dialog: an icon tile, a title and an optional subtitle.
fn header(ui: &mut egui::Ui, glyph: &str, color: egui::Color32, title: &str, subtitle: Option<&str>) {
    let palette = Palette::of(ui);
    Frame::new().inner_margin(Margin { left: 24, right: 24, top: 22, bottom: 14 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            Frame::new()
                .fill(color.gamma_multiply(0.15))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(Margin::same(9))
                .show(ui, |ui| ui.label(RichText::new(glyph).size(20.0).color(color)));
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(theme::strong(title, 17.0).color(palette.text_strong));
                if let Some(subtitle) = subtitle {
                    ui.label(RichText::new(subtitle).color(palette.text_weak));
                }
            });
        });
    });
}

/// The bottom bar of a dialog: `left` on the left, `right` laid out from the right edge.
fn footer(ui: &mut egui::Ui, left: impl FnOnce(&mut egui::Ui), right: impl FnOnce(&mut egui::Ui)) {
    let palette = Palette::of(ui);
    let top = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, palette.border));
    Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            left(ui);
            ui.with_layout(Layout::right_to_left(Align::Center), right);
        });
    });
}

fn cancel(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    if ui.add(egui::Button::new("Cancel").frame_when_inactive(false)).clicked() {
        actions.push(Action::CloseDialog);
    }
}

/// One line of the settings dialog: a label on the left, the control on the right.
fn setting(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let palette = Palette::of(ui);
        ui.allocate_ui_with_layout(egui::vec2(LABEL_WIDTH, 28.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(LABEL_WIDTH);
            ui.label(RichText::new(label).color(palette.text));
        });
        add(ui);
    });
}

/// A titled card in the settings dialog.
fn group(ui: &mut egui::Ui, glyph: &str, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let palette = Palette::of(ui);
    theme::card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(theme::icon(glyph, palette.accent));
            ui.label(theme::strong(title, 14.0).color(palette.text_strong));
        });
        ui.add_space(2.0);
        add(ui);
    });
}

fn settings_dialog(ui: &mut egui::Ui, state: &AppState, draft: &mut Settings, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.set_width(640.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    header(ui, icon::GEAR_SIX, palette.accent, "Settings", Some("Changes apply when you save."));
    let id = ui.make_persistent_id("settings-tab");
    let mut tab = ui.data(|data| data.get_temp::<SettingsTab>(id)).unwrap_or_default();
    Frame::new().inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 16 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        let tabs = [
            (SettingsTab::General, icon::GEAR_SIX, "General"),
            (SettingsTab::Storage, icon::DATABASE, "Storage"),
            (SettingsTab::Protobuf, icon::BROADCAST, "Protobuf"),
        ];
        if let Some(chosen) = theme::tabs(ui, tab, &tabs) {
            tab = chosen;
        }
        ScrollArea::vertical().id_salt(("settings", tab as u8)).max_height(440.0).auto_shrink([false, true]).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                match tab {
                    SettingsTab::General => general_settings(ui, draft),
                    SettingsTab::Storage => storage_settings(ui, draft, actions),
                    SettingsTab::Protobuf => protobuf_settings(ui, state, actions),
                }
            },
        );
    });
    ui.data_mut(|data| data.insert_temp(id, tab));
    footer(
        ui,
        |ui| {
            ui.weak(concat!("MQTT Viewer ", env!("CARGO_PKG_VERSION")));
            ui.hyperlink_to("Website", env!("CARGO_PKG_REPOSITORY"));
        },
        |ui| {
            if theme::add_filled(ui, theme::primary_button(ui, Some(icon::FLOPPY_DISK), "Save"), "Save").clicked() {
                actions.push(Action::ApplySettings);
            }
            cancel(ui, actions);
        },
    );
}

fn general_settings(ui: &mut egui::Ui, draft: &mut Settings) {
    group(ui, icon::PLUGS_CONNECTED, "Startup", |ui| {
        ui.checkbox(&mut draft.connect_on_start, "Connect to the last used broker when the app starts");
    });
    group(ui, icon::GEAR_SIX, "Appearance", |ui| {
        setting(ui, "Theme", |ui| {
            let themes = [(Theme::System, "System"), (Theme::Light, "Light"), (Theme::Dark, "Dark")];
            theme::segmented_value(ui, &mut draft.theme, &themes);
        });
        setting(ui, "Timestamps", |ui| {
            let formats = [
                (TimestampFormat::Time, "Time"),
                (TimestampFormat::DateTime, "Date and time"),
                (TimestampFormat::Iso, "ISO 8601"),
                (TimestampFormat::Relative, "Relative"),
            ];
            theme::segmented_value(ui, &mut draft.timestamp_format, &formats);
        });
    });
    group(ui, icon::TREE_STRUCTURE, "Topic tree", |ui| {
        setting(ui, "Topic order", |ui| {
            let orders = [
                (TreeOrder::Alphabetical, "A–Z"),
                (TreeOrder::Messages, "Most messages"),
                (TreeOrder::Topics, "Most topics"),
            ];
            theme::segmented_value(ui, &mut draft.tree_order, &orders);
        });
        setting(ui, "Expand new topics", |ui| {
            ui.add(DragValue::new(&mut draft.auto_expand_depth).range(0..=32).suffix(" levels"))
                .accessible_name("Expand new topics");
            ui.weak("0 leaves the tree collapsed");
        });
        setting(ui, "Activity", |ui| {
            ui.checkbox(&mut draft.highlight_activity, "Highlight topics when a message arrives");
        });
    });
}

fn storage_settings(ui: &mut egui::Ui, draft: &mut Settings, actions: &mut Vec<Action>) {
    group(ui, icon::DATABASE, "Memory", |ui| {
        setting(ui, "History per topic", |ui| {
            ui.add(DragValue::new(&mut draft.history_cap).range(1..=100_000).suffix(" messages"))
                .accessible_name("History per topic");
        });
        setting(ui, "Memory budget", |ui| {
            ui.add(DragValue::new(&mut draft.memory_budget_mb).range(8..=65_536).suffix(" MiB"))
                .accessible_name("Memory budget");
        });
    });
    group(ui, icon::FLOPPY_DISK, "History on disk", |ui| {
        ui.checkbox(&mut draft.persist_history, "Store received messages in a local database");
        ui.add_enabled_ui(draft.persist_history, |ui| {
            setting(ui, "Keep for", |ui| {
                ui.add(DragValue::new(&mut draft.retention_days).range(0..=3650).suffix(" days"))
                    .accessible_name("Retention days");
                ui.label("and at most");
                ui.add(DragValue::new(&mut draft.retention_mb).range(0..=1_048_576).suffix(" MiB"))
                    .accessible_name("Retention size");
            });
            ui.weak("0 disables a limit.");
        });
        if theme::icon_button(ui, icon::TRASH, "Clear stored history").clicked() {
            actions.push(Action::ClearStoredHistory);
        }
    });
}

fn protobuf_settings(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    group(ui, icon::BROADCAST, "Schemas", |ui| proto_files(ui, state, actions));
    group(ui, icon::HASH, "Topic bindings", |ui| proto_bindings(ui, state, actions));
}

fn proto_files(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    if state.config.proto_files.is_empty() {
        ui.weak("Load .proto files to decode Protobuf payloads.");
    }
    for (index, file) in state.config.proto_files.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.monospace(file.display().to_string());
            if theme::icon_only(ui, icon::X, "Remove schema").clicked() {
                actions.push(Action::RemoveProtoFile(index));
            }
        });
    }
    if theme::icon_button(ui, icon::PLUS, "Add .proto file…").clicked()
        && let Some(path) = state.files.open("Protobuf", &["proto"])
    {
        actions.push(Action::AddProtoFile(path));
    }
}

fn proto_bindings(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    for (index, binding) in state.config.proto_bindings.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.monospace(&binding.pattern);
            ui.weak("as");
            ui.monospace(&binding.message_type);
            if theme::icon_only(ui, icon::X, "Remove binding").clicked() {
                actions.push(Action::RemoveProtoBinding(index));
            }
        });
    }
    let id = ui.scope_id().with("new-binding");
    let mut binding = ui.data_mut(|data| data.get_temp::<(String, String)>(id).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add(
            TextEdit::singleline(&mut binding.0)
                .hint_text("devices/+/telemetry")
                .margin(egui::vec2(8.0, 5.0))
                .desired_width(200.0),
        )
        .accessible_name("Binding topic filter");
        let types = state.decoder.registry().message_types();
        let label = if binding.1.is_empty() { "message type" } else { binding.1.as_str() };
        egui::ComboBox::from_id_salt("binding-type")
            .selected_text(label.to_owned())
            .show_ui(ui, |ui| {
                if types.is_empty() {
                    ui.weak("Load a .proto file first");
                }
                for name in types {
                    ui.selectable_value(&mut binding.1, name.clone(), name);
                }
            })
            .response
            .accessible_name("Message type");
        if ui.button("Bind").clicked() {
            let (pattern, message_type) = std::mem::take(&mut binding);
            actions.push(Action::AddProtoBinding(ProtoBinding { pattern, message_type }));
        }
    });
    ui.data_mut(|data| data.insert_temp(id, binding));
}

fn confirm_delete(ui: &mut egui::Ui, topics: &[String], actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.set_width(480.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    header(ui, icon::TRASH, palette.error, "Delete retained messages", None);
    Frame::new().inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 16 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.label(format!(
            "This clears the retained message of {} on the broker:",
            crate::format::plural(topics.len() as u64, "topic")
        ));
        Frame::new()
            .fill(palette.sunken)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for topic in topics.iter().take(200) {
                        ui.monospace(topic);
                    }
                    if topics.len() > 200 {
                        ui.weak(format!("… and {} more", topics.len() - 200));
                    }
                });
            });
    });
    footer(
        ui,
        |_| {},
        |ui| {
            if theme::add_filled(ui, theme::danger_button(ui, Some(icon::TRASH), "Delete"), "Delete").clicked() {
                actions.push(Action::ConfirmDelete);
            }
            cancel(ui, actions);
        },
    );
}

fn replay_dialog(ui: &mut egui::Ui, state: &AppState, dialog: &mut ReplayDialog, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.set_width(520.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    let steps = record::plan_replay(&dialog.messages, &dialog.options);
    let summary = format!(
        "{} messages, {:.1} s at the chosen speed",
        dialog.messages.len(),
        record::replay_duration(&steps).as_secs_f64()
    );
    header(ui, icon::PLAY_CIRCLE, palette.accent, "Replay recording", Some(&summary));
    let target = state.profile.as_ref().filter(|_| state.connection.is_active()).map(ConnectionProfile::url);
    Frame::new().inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 16 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.label(RichText::new(dialog.path.display().to_string()).monospace().color(palette.text_weak));
        theme::field(ui, "Speed", |ui| {
            ui.horizontal(|ui| {
                ui.add(DragValue::new(&mut dialog.options.speed).range(0.0..=1000.0).speed(0.1).suffix("×"))
                    .accessible_name("Replay speed");
                ui.weak("0 sends everything at once");
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let width = (ui.available_width() - 12.0) / 2.0;
            theme::field(ui, "Replace topic prefix", |ui| {
                ui.add(
                    TextEdit::singleline(&mut dialog.options.prefix_from)
                        .hint_text("home/")
                        .margin(egui::vec2(8.0, 5.0))
                        .desired_width(width),
                )
                .accessible_name("Prefix to replace");
            });
            theme::field(ui, "with", |ui| {
                ui.add(
                    TextEdit::singleline(&mut dialog.options.prefix_to)
                        .hint_text("sandbox/home/")
                        .margin(egui::vec2(8.0, 5.0))
                        .desired_width(width),
                )
                .accessible_name("Replacement prefix");
            });
        });
        let (glyph, color, text) = target.as_ref().map_or_else(
            || (icon::PLUGS, palette.error, "Not connected to a broker".to_owned()),
            |url| (icon::WARNING, palette.warn, format!("The messages will be published to {url}")),
        );
        ui.horizontal(|ui| {
            ui.label(theme::icon(glyph, color));
            ui.label(RichText::new(text).color(color));
        });
    });
    footer(
        ui,
        |_| {},
        |ui| {
            let start = theme::primary_button(ui, Some(icon::PLAY), "Start replay");
            let response = ui.add_enabled(target.is_some(), start).accessible_name("Start replay");
            if response.clicked() {
                actions.push(Action::StartReplay);
            }
            cancel(ui, actions);
        },
    );
}
