//! The right-hand panel: topic, value, history, publish and statistics for the selected topic.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, CollapsingHeader, FontId, RichText, ScrollArea, TextEdit, TextStyle};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mqtt_viewer_core::decode::{self, Body, Format};
use mqtt_viewer_core::diff::ChangeKind;
use mqtt_viewer_core::message::QoS;
use mqtt_viewer_core::tree::NodeId;

use super::theme::Palette;
use crate::format;
use crate::forms::PayloadMode;
use crate::highlight::{self, Token};
use crate::state::{Action, AppState, ExportFormat, ExportScope, ValueMode};

/// History rows drawn at most; the newest are shown.
const MAX_HISTORY_ROWS: usize = 200;

pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let selected = state.selected.filter(|id| state.tree.contains(*id));
        CollapsingHeader::new("Topic").default_open(true).show(ui, |ui| match selected {
            Some(node) => topic(ui, state, node, actions),
            None => {
                ui.weak("Select a topic in the tree");
            }
        });
        if let Some(node) = selected {
            CollapsingHeader::new("Value").default_open(true).show(ui, |ui| value(ui, state, node, actions));
            CollapsingHeader::new("History").default_open(false).show(ui, |ui| history(ui, state, node, actions));
        }
        CollapsingHeader::new("Publish").default_open(true).show(ui, |ui| publish(ui, state, actions));
        CollapsingHeader::new("Stats").default_open(false).show(ui, |ui| stats(ui, state, selected));
    });
}

fn topic(ui: &mut egui::Ui, state: &AppState, node: NodeId, actions: &mut Vec<Action>) {
    let full = state.tree.topic(node);
    // Breadcrumb: every level selects that ancestor.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut ancestors = vec![node];
        while let Some(parent) = ancestors.last().and_then(|id| state.tree.node(*id).parent()) {
            ancestors.push(parent);
        }
        // Drop the invisible root.
        ancestors.pop();
        for (index, id) in ancestors.iter().rev().enumerate() {
            if index > 0 {
                ui.weak("/");
            }
            let name = state.tree.node(*id).name();
            if ui.link(if name.is_empty() { "(empty)" } else { name }).clicked() {
                actions.push(Action::Select(Some(*id)));
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Copy topic").clicked() {
            ui.ctx().copy_text(full.clone());
        }
        let has_value = state.tree.node(node).last().is_some();
        if ui.add_enabled(has_value, egui::Button::new("Delete")).on_hover_text("Clear the retained message").clicked()
        {
            actions.push(Action::DeleteRetained { node, recursive: false });
        }
        let has_children = !state.tree.node(node).children().is_empty();
        if ui.add_enabled(has_children, egui::Button::new("Delete all below…")).clicked() {
            actions.push(Action::DeleteRetained { node, recursive: true });
        }
        ui.menu_button("Export…", |ui| {
            let scopes = [
                ("Latest value", ExportScope::Value),
                ("History", ExportScope::History),
                ("Subtree values", ExportScope::Subtree),
            ];
            for (label, scope) in scopes {
                for (extension, export_format) in [("json", ExportFormat::Json), ("csv", ExportFormat::Csv)] {
                    if ui.button(format!("{label} as {}", extension.to_uppercase())).clicked() {
                        ui.close();
                        let dialog = rfd::FileDialog::new().set_file_name(format!("export.{extension}"));
                        if let Some(path) = dialog.add_filter(extension, &[extension]).save_file() {
                            actions.push(Action::Export { scope, format: export_format, path });
                        }
                    }
                }
            }
        });
    });
}

fn value(ui: &mut egui::Ui, state: &mut AppState, node: NodeId, actions: &mut Vec<Action>) {
    let Some(entry) = state.tree.node(node).last().cloned() else {
        ui.weak("No message on this exact topic; pick one of its subtopics");
        return;
    };
    let palette = Palette::of(ui);
    let topic = state.tree.topic(node);
    let requested = state.format_for(&topic);
    let style = state.config.settings.timestamp_format;

    ui.horizontal_wrapped(|ui| {
        ui.label(format!("QoS {}", entry.qos.level()));
        if entry.retain {
            ui.label(RichText::new("retained").color(palette.warn));
        }
        ui.label(format::bytes(entry.payload.len()));
        ui.label(format::timestamp(entry.received_at, style, Timestamp::now(), &TimeZone::system()))
            .on_hover_text(entry.received_at.to_string());
    });
    ui.horizontal(|ui| {
        let modes = [(ValueMode::Decoded, "Decoded"), (ValueMode::Raw, "Raw"), (ValueMode::Diff, "Diff")];
        for (mode, label) in modes {
            if ui.selectable_label(state.value_mode == mode, label).clicked() {
                actions.push(Action::SetValueMode(mode));
            }
        }
        ui.separator();
        let applied = state.selected_value().map_or(requested, |view| view.decoded.format);
        let label = if requested == Format::Auto {
            format!("Auto ({})", applied.label())
        } else {
            requested.label().to_owned()
        };
        egui::ComboBox::from_id_salt("decoder")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for option in Format::ALL {
                    if ui.selectable_label(requested == option, option.label()).clicked() {
                        actions.push(Action::SetDecoder { pattern: topic.clone(), format: option });
                    }
                }
            })
            .response
            .accessible_name("Decoder");
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(String::from_utf8_lossy(&entry.payload).into_owned());
        }
    });

    let mono = TextStyle::Monospace.resolve(ui.style());
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ScrollArea::both().id_salt("value").max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
            match state.value_mode {
                ValueMode::Decoded => decoded(ui, state, &mono, &palette, actions),
                ValueMode::Raw => {
                    ui.add(
                        egui::Label::new(RichText::new(decode::hex_dump(&entry.payload)).font(mono.clone())).extend(),
                    );
                }
                ValueMode::Diff => diff(ui, state, &mono, &palette),
            }
        });
    });

    if let Some(properties) = &entry.properties {
        CollapsingHeader::new("MQTT 5 properties").default_open(true).show(ui, |ui| {
            egui::Grid::new("properties").num_columns(2).show(ui, |ui| {
                let mut row = |key: &str, text: String| {
                    ui.weak(key);
                    ui.label(text);
                    ui.end_row();
                };
                if let Some(content_type) = &properties.content_type {
                    row("Content type", content_type.clone());
                }
                if let Some(response_topic) = &properties.response_topic {
                    row("Response topic", response_topic.clone());
                }
                if let Some(data) = &properties.correlation_data {
                    row("Correlation data", decode::preview(data, 80));
                }
                if let Some(expiry) = properties.message_expiry_secs {
                    row("Message expiry", format!("{expiry} s"));
                }
                if let Some(is_utf8) = properties.payload_is_utf8 {
                    row("Payload format", if is_utf8 { "UTF-8 text" } else { "bytes" }.to_owned());
                }
                for (key, text) in &properties.user_properties {
                    row(key, text.clone());
                }
            });
        });
    }
}

fn decoded(ui: &mut egui::Ui, state: &mut AppState, mono: &FontId, palette: &Palette, actions: &mut Vec<Action>) {
    let text_color = ui.visuals().text_color();
    let Some(topic) = state.selected_topic() else { return };
    let Some(view) = state.selected_value() else { return };
    let color = |token: Token| match token {
        Token::Key => palette.key,
        Token::String => palette.string,
        Token::Number => palette.number,
        Token::Literal => palette.literal,
        Token::Punctuation | Token::Other => text_color,
    };
    let mut job = LayoutJob::default();
    match &view.decoded.body {
        Body::Json(_) => {
            for (token, range) in highlight::json_tokens(&view.text) {
                job.append(
                    &view.text[range],
                    0.0,
                    TextFormat { font_id: mono.clone(), color: color(token), ..Default::default() },
                );
            }
        }
        Body::Error(_) => {
            job.append(
                &view.text,
                0.0,
                TextFormat { font_id: mono.clone(), color: palette.error, ..Default::default() },
            );
        }
        Body::Text(_) | Body::Hex(_) => {
            job.append(&view.text, 0.0, TextFormat { font_id: mono.clone(), color: text_color, ..Default::default() });
        }
    }
    ui.add(egui::Label::new(job).extend());
    if view.truncated {
        ui.weak("Output truncated; copy the value to get all of it");
    }
    if !view.chartable.is_empty() {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.weak("Chart:");
            for pointer in &view.chartable {
                let label = if pointer.is_empty() { "value" } else { pointer.as_str() };
                if ui.small_button(label).on_hover_text("Add this number to the chart panel").clicked() {
                    actions.push(Action::AddChart { topic: topic.clone(), pointer: pointer.clone() });
                }
            }
        });
    }
}

fn diff(ui: &mut egui::Ui, state: &mut AppState, mono: &FontId, palette: &Palette) {
    let text_color = ui.visuals().text_color();
    let explicit = state.compare_target().is_some();
    let Some(lines) = state.selected_diff() else {
        ui.weak("There is no earlier message to compare with");
        return;
    };
    let mut job = LayoutJob::default();
    for line in lines {
        let (prefix, color) = match line.kind {
            ChangeKind::Equal => ("  ", text_color),
            ChangeKind::Added => ("+ ", palette.added),
            ChangeKind::Removed => ("- ", palette.removed),
        };
        job.append(
            &format!("{prefix}{}\n", line.text),
            0.0,
            TextFormat { font_id: mono.clone(), color, ..Default::default() },
        );
    }
    ui.add(egui::Label::new(job).extend());
    ui.weak(if explicit { "Compared with the selected history entry" } else { "Compared with the previous message" });
}

fn history(ui: &mut egui::Ui, state: &AppState, node: NodeId, actions: &mut Vec<Action>) {
    let entries = state.tree.node(node).history();
    if entries.is_empty() {
        ui.weak("No messages");
        return;
    }
    let style = state.config.settings.timestamp_format;
    let now = Timestamp::now();
    let zone = TimeZone::system();
    ui.weak(format!(
        "{} stored of {} received",
        format::count(entries.len() as u64),
        format::count(state.tree.node(node).message_count())
    ));
    ScrollArea::vertical().id_salt("history").max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
        for entry in entries.iter().rev().take(MAX_HISTORY_ROWS) {
            ui.horizontal(|ui| {
                ui.weak(format::timestamp(entry.received_at, style, now, &zone));
                let selected = state.compare_target() == Some(entry.seq());
                let preview = decode::preview(&entry.payload, 60);
                let label = ui.selectable_label(selected, RichText::new(preview).monospace());
                if label.on_hover_text("Compare the latest value with this message").clicked() {
                    actions.push(Action::CompareWith((!selected).then_some(entry.seq())));
                }
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(String::from_utf8_lossy(&entry.payload).into_owned());
                }
            });
        }
    });
}

fn publish(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.label("Topic");
        ui.add(
            TextEdit::singleline(&mut state.publish.topic).hint_text("topic/to/publish").desired_width(f32::INFINITY),
        )
        .accessible_name("Publish topic");
    });
    ui.horizontal(|ui| {
        for mode in PayloadMode::ALL {
            ui.selectable_value(&mut state.publish.mode, mode, mode.label());
        }
        if state.publish.mode == PayloadMode::Json && ui.button("Format").clicked() {
            actions.push(Action::FormatPublishPayload);
        }
    });
    ui.add(
        TextEdit::multiline(&mut state.publish.payload)
            .font(TextStyle::Monospace)
            .desired_rows(5)
            .desired_width(f32::INFINITY)
            .hint_text("payload"),
    )
    .accessible_name("Publish payload");
    if let Err(problem) = state.publish.check_payload() {
        ui.label(RichText::new(problem).color(palette.error));
    }
    ui.horizontal(|ui| {
        ui.label("QoS");
        for qos in QoS::ALL {
            ui.selectable_value(&mut state.publish.qos, qos, qos.level().to_string());
        }
        ui.checkbox(&mut state.publish.retain, "retain");
        if ui.button(RichText::new("Publish").strong()).clicked() {
            actions.push(Action::Publish);
        }
    });

    publish_properties(ui, state);
    publish_presets(ui, state, actions);
}

fn publish_properties(ui: &mut egui::Ui, state: &mut AppState) {
    CollapsingHeader::new("MQTT 5 properties").id_salt("publish-properties").show(ui, |ui| {
        egui::Grid::new("publish-properties-grid").num_columns(2).show(ui, |ui| {
            ui.label("Content type");
            ui.text_edit_singleline(&mut state.publish.content_type).accessible_name("Content type");
            ui.end_row();
            ui.label("Response topic");
            ui.text_edit_singleline(&mut state.publish.response_topic).accessible_name("Response topic");
            ui.end_row();
            ui.label("Correlation data");
            ui.text_edit_singleline(&mut state.publish.correlation_data).accessible_name("Correlation data");
            ui.end_row();
            ui.label("Expiry (s)");
            ui.text_edit_singleline(&mut state.publish.expiry_secs).accessible_name("Message expiry");
            ui.end_row();
        });
        ui.checkbox(&mut state.publish.mark_utf8, "Mark payload as UTF-8 text");
        ui.label("User properties");
        let mut remove = None;
        for (index, (key, text)) in state.publish.user_properties.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(key).hint_text("key").desired_width(120.0)).accessible_name("Property key");
                ui.add(TextEdit::singleline(text).hint_text("value").desired_width(160.0))
                    .accessible_name("Property value");
                if ui.small_button("×").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            state.publish.user_properties.remove(index);
        }
        if ui.small_button("Add property").clicked() {
            state.publish.user_properties.push((String::new(), String::new()));
        }
        ui.weak("Properties are only sent on MQTT 5 connections");
    });
}

fn publish_presets(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.menu_button("Sent", |ui| {
            if state.publish_history.is_empty() {
                ui.weak("Nothing published yet");
            }
            for (index, request) in state.publish_history.iter().enumerate() {
                let label = format!("{}  {}", request.topic, decode::preview(&request.payload, 30));
                if ui.button(label).clicked() {
                    actions.push(Action::LoadPublishHistory(index));
                    ui.close();
                }
            }
        });
        ui.menu_button("Presets", |ui| {
            if state.config.publish_presets.is_empty() {
                ui.weak("No presets saved");
            }
            for (index, preset) in state.config.publish_presets.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button(&preset.name).clicked() {
                        actions.push(Action::LoadPreset(index));
                        ui.close();
                    }
                    if ui.small_button("×").on_hover_text("Delete preset").clicked() {
                        actions.push(Action::DeletePreset(index));
                    }
                });
            }
        });
        let name_id = ui.scope_id().with("preset-name");
        let mut name = ui.data_mut(|data| data.get_temp::<String>(name_id).unwrap_or_default());
        ui.add(TextEdit::singleline(&mut name).hint_text("preset name").desired_width(120.0))
            .accessible_name("Preset name");
        if ui.button("Save preset").clicked() {
            actions.push(Action::SavePreset(std::mem::take(&mut name)));
        }
        ui.data_mut(|data| data.insert_temp(name_id, name));
    });
}

fn stats(ui: &mut egui::Ui, state: &AppState, selected: Option<NodeId>) {
    let totals = state.tree.stats();
    egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
        let mut row = |key: &str, text: String| {
            ui.weak(key);
            ui.label(text);
            ui.end_row();
        };
        row("Topics", format::count(totals.topics as u64));
        row("Messages", format::count(totals.messages));
        row("Rate", format::rate(state.message_rate()));
        row("Stored messages", format::count(totals.stored_messages as u64));
        row("History memory", format::bytes(totals.stored_bytes));
        if let Some(node) = selected.map(|id| state.tree.node(id)) {
            row("Selected: messages", format::count(node.message_count()));
            row("Selected: subtopics", format::count(node.subtree_topics() as u64));
            row("Selected: messages below", format::count(node.subtree_messages()));
        }
    });
}
