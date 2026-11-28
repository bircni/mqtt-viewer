//! The right-hand panel: topic, value, history, publish and statistics for the selected topic.

use eframe::egui::collapsing_header::CollapsingState;
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Atom, CollapsingHeader, FontId, RichText, ScrollArea, TextEdit, TextStyle};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mqtt_viewer_core::decode::{self, Body, Format};
use mqtt_viewer_core::diff::ChangeKind;
use mqtt_viewer_core::message::QoS;
use mqtt_viewer_core::tree::NodeId;

use super::fonts::icon;
use super::theme::{self, Palette};
use crate::format;
use crate::forms::PayloadMode;
use crate::highlight::{self, Token};
use crate::state::{Action, AppState, ExportFormat, ExportScope, ValueMode};

/// History rows drawn at most; the newest are shown.
const MAX_HISTORY_ROWS: usize = 200;

pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        let selected = state.selected.filter(|id| state.tree.contains(*id));
        section(ui, icon::HASH, "Topic", true, |ui| match selected {
            Some(node) => topic(ui, state, node, actions),
            None => {
                ui.weak("Select a topic in the tree");
            }
        });
        if let Some(node) = selected {
            section(ui, icon::BROADCAST, "Value", true, |ui| value(ui, state, node, actions));
            section(ui, icon::CLOCK_COUNTER_CLOCKWISE, "History", false, |ui| history(ui, state, node, actions));
        }
        section(ui, icon::PAPER_PLANE_TILT, "Publish", true, |ui| publish(ui, state, actions));
        section(ui, icon::CHART_LINE, "Stats", false, |ui| stats(ui, state, selected));
    });
}

/// One collapsible section of the sidebar, drawn as a card whose whole header toggles it.
fn section(ui: &mut egui::Ui, glyph: &str, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    let palette = Palette::of(ui);
    theme::card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let id = ui.make_persistent_id(("section", title));
        let mut collapsing = CollapsingState::load_with_default_open(ui.ctx(), id, open);
        let caret = if collapsing.is_open() { icon::CARET_DOWN } else { icon::CARET_RIGHT };
        let header = egui::Button::new((
            theme::icon(glyph, palette.accent),
            theme::strong(title, 13.5).color(palette.text_strong),
            Atom::grow(),
            RichText::new(caret).size(12.0).color(palette.text_weak),
        ))
        .frame(false)
        .min_size(egui::vec2(ui.available_width(), 22.0));
        let response = ui.add(header);
        let open_now = collapsing.is_open();
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, true, title);
            info.selected = Some(open_now);
            info
        });
        if response.clicked() {
            collapsing.toggle(ui);
        }
        collapsing.show_body_unindented(ui, |ui| {
            ui.add_space(4.0);
            body(ui);
        });
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
            let text = theme::strong(if name.is_empty() { "(empty)" } else { name }, 14.0);
            let last = index + 1 == ancestors.len();
            let text = text.color(if last { Palette::of(ui).text_strong } else { Palette::of(ui).accent });
            if ui.add(egui::Link::new(text)).clicked() {
                actions.push(Action::Select(Some(*id)));
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if theme::icon_button(ui, icon::COPY, "Copy topic").clicked() {
            ui.ctx().copy_text(full.clone());
        }
        let has_value = state.tree.node(node).last().is_some();
        let delete = ui
            .add_enabled_ui(has_value, |ui| theme::icon_button(ui, icon::TRASH, "Delete"))
            .inner
            .on_hover_text("Clear the retained message");
        if delete.clicked() {
            actions.push(Action::DeleteRetained { node, recursive: false });
        }
        let has_children = !state.tree.node(node).children().is_empty();
        let below =
            ui.add_enabled_ui(has_children, |ui| theme::icon_button(ui, icon::TRASH, "Delete all below…")).inner;
        if below.clicked() {
            actions.push(Action::DeleteRetained { node, recursive: true });
        }
        let weak = Palette::of(ui).text_weak;
        ui.menu_button((theme::icon(icon::EXPORT, weak), "Export…"), |ui| {
            let scopes = [
                ("Latest value", ExportScope::Value),
                ("History", ExportScope::History),
                ("Subtree values", ExportScope::Subtree),
            ];
            for (label, scope) in scopes {
                for (extension, export_format) in [("json", ExportFormat::Json), ("csv", ExportFormat::Csv)] {
                    if ui.button(format!("{label} as {}", extension.to_uppercase())).clicked() {
                        ui.close();
                        if let Some(path) = state.files.save(&format!("export.{extension}"), extension, &[extension]) {
                            actions.push(Action::Export { scope, format: export_format, path });
                        }
                    }
                }
            }
        })
        .response
        .accessible_name("Export…");
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
        ui.spacing_mut().item_spacing.x = 6.0;
        let neutral = ui.visuals().text_color();
        theme::badge(ui, format!("QoS {}", entry.qos.level()), palette.accent);
        if entry.retain {
            theme::badge(ui, "retained", palette.warn);
        }
        theme::badge(ui, format::bytes(entry.payload.len()), neutral);
        theme::badge(ui, format::timestamp(entry.received_at, style, Timestamp::now(), &TimeZone::system()), neutral)
            .on_hover_text(entry.received_at.to_string());
    });
    ui.horizontal(|ui| {
        let modes = [(ValueMode::Decoded, "Decoded"), (ValueMode::Raw, "Raw"), (ValueMode::Diff, "Diff")];
        if let Some(mode) = theme::segmented(ui, state.value_mode, &modes) {
            actions.push(Action::SetValueMode(mode));
        }
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_only(ui, icon::COPY, "Copy").clicked() {
                ui.ctx().copy_text(String::from_utf8_lossy(&entry.payload).into_owned());
            }
        });
    });

    let mono = TextStyle::Monospace.resolve(ui.style());
    let mode = state.value_mode;
    egui::Frame::new()
        .fill(ui.visuals().code_bg_color)
        .stroke(egui::Stroke::new(1.0, palette.border))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ScrollArea::both().id_salt("value").max_height(360.0).auto_shrink([false, true]).show(
                ui,
                |ui| match mode {
                    ValueMode::Decoded => decoded(ui, state, &mono, &palette, actions),
                    ValueMode::Raw => {
                        ui.add(
                            egui::Label::new(RichText::new(decode::hex_dump(&entry.payload)).font(mono.clone()))
                                .extend(),
                        );
                    }
                    ValueMode::Diff => diff(ui, state, &mono, &palette),
                },
            );
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
    let (added, removed) = mqtt_viewer_core::diff::summary(lines);
    let height = ui.fonts_mut(|fonts| fonts.row_height(mono)) + 2.0;
    let width = ui.available_width();
    ui.spacing_mut().item_spacing.y = 0.0;
    for line in lines {
        let (marker, color, background) = match line.kind {
            ChangeKind::Equal => (" ", text_color, egui::Color32::TRANSPARENT),
            ChangeKind::Added => ("+", palette.added, palette.added.gamma_multiply(0.14)),
            ChangeKind::Removed => ("−", palette.removed, palette.removed.gamma_multiply(0.14)),
        };
        let galley = ui.painter().layout_no_wrap(format!("{marker} {}", line.text), mono.clone(), color);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(width.max(galley.size().x + 8.0), height), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, background);
        ui.painter().galley(egui::pos2(rect.left() + 4.0, rect.top() + 1.0), galley, color);
    }
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.weak(if explicit {
            "Compared with the selected history entry:"
        } else {
            "Compared with the previous message:"
        });
        ui.label(RichText::new(format!("+{}", format::plural(added as u64, "line"))).color(palette.added));
        ui.label(RichText::new(format!("−{}", format::plural(removed as u64, "line"))).color(palette.removed));
    });
}

fn history(ui: &mut egui::Ui, state: &AppState, node: NodeId, actions: &mut Vec<Action>) {
    let entries = state.tree.node(node).history();
    if entries.is_empty() {
        ui.weak("No messages");
        return;
    }
    history_chart(ui, entries, &Palette::of(ui));
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
                if theme::icon_only(ui, icon::COPY, "Copy").clicked() {
                    ui.ctx().copy_text(String::from_utf8_lossy(&entry.payload).into_owned());
                }
            });
        }
    });
}

/// A small chart of the history when the topic carries plain numbers.
fn history_chart(
    ui: &mut egui::Ui,
    entries: &std::collections::VecDeque<mqtt_viewer_core::tree::HistoryEntry>,
    palette: &Palette,
) {
    let points: Vec<[f64; 2]> = entries
        .iter()
        .filter_map(|entry| {
            let value = std::str::from_utf8(&entry.payload).ok().and_then(mqtt_viewer_core::numeric::parse_number)?;
            #[expect(clippy::cast_precision_loss, reason = "millisecond timestamps fit into an f64 exactly")]
            let seconds = entry.received_at.as_millisecond() as f64 / 1000.0;
            Some([seconds, value])
        })
        .collect();
    if points.len() < 2 || points.len() * 2 < entries.len() {
        return;
    }
    egui_plot::Plot::new("history-chart")
        .height(110.0)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .show_x(false)
        .x_axis_formatter(|_, _| String::new())
        .show(ui, |plot_ui| {
            let line = egui_plot::Line::new("history", egui_plot::PlotPoints::from(points.clone()));
            plot_ui.line(line.color(palette.warn).width(1.75));
            plot_ui.points(egui_plot::Points::new("samples", points).radius(2.5).color(palette.warn));
        });
}

fn publish(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    theme::field(ui, "Topic", |ui| {
        ui.add(
            TextEdit::singleline(&mut state.publish.topic)
                .hint_text("topic/to/publish")
                .font(TextStyle::Monospace)
                .margin(egui::vec2(8.0, 5.0))
                .desired_width(f32::INFINITY),
        )
        .accessible_name("Publish topic");
    });
    theme::field(ui, "Payload", |ui| {
        ui.horizontal(|ui| {
            let modes = PayloadMode::ALL.map(|mode| (mode, mode.label()));
            theme::segmented_value(ui, &mut state.publish.mode, &modes);
            if state.publish.mode == PayloadMode::Json && ui.button("Format").clicked() {
                actions.push(Action::FormatPublishPayload);
            }
        });
        ui.add(
            TextEdit::multiline(&mut state.publish.payload)
                .font(TextStyle::Monospace)
                .margin(egui::vec2(8.0, 6.0))
                .desired_rows(5)
                .desired_width(f32::INFINITY)
                .hint_text("payload"),
        )
        .accessible_name("Publish payload");
    });
    if let Err(problem) = state.publish.check_payload() {
        ui.label(RichText::new(problem).color(palette.error));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("QoS").color(palette.text_weak));
        let levels = QoS::ALL.map(|qos| (qos, ["0", "1", "2"][usize::from(qos.level())]));
        theme::segmented_value(ui, &mut state.publish.qos, &levels);
        ui.checkbox(&mut state.publish.retain, "retain");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = theme::primary_button(ui, Some(icon::PAPER_PLANE_TILT), "Publish");
            if theme::add_filled(ui, button, "Publish").clicked() {
                actions.push(Action::Publish);
            }
        });
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
                if theme::icon_only(ui, icon::X, "Remove property").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            state.publish.user_properties.remove(index);
        }
        if theme::icon_button(ui, icon::PLUS, "Add property").clicked() {
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
                    if theme::icon_only(ui, icon::X, "Delete preset").clicked() {
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
    let palette = Palette::of(ui);
    let heading = |ui: &mut egui::Ui, text: &str| {
        ui.label(RichText::new(text).font(super::fonts::semibold(11.5)).color(palette.text_weak));
    };
    let grid = |ui: &mut egui::Ui, id: &str, rows: Vec<(String, String)>| {
        egui::Grid::new(id).num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            for (key, text) in rows {
                ui.label(RichText::new(key).color(palette.text_weak));
                ui.label(text);
                ui.end_row();
            }
        });
    };
    if let Some(profile) = state.profile.as_ref().filter(|_| state.connection.is_active()) {
        heading(ui, "CONNECTION");
        grid(
            ui,
            "connection",
            vec![
                ("Broker".to_owned(), profile.url()),
                ("Client ID".to_owned(), profile.client_id.clone()),
                ("Protocol".to_owned(), format!("MQTT {}", profile.protocol.label())),
            ],
        );
    }
    let broker = mqtt_viewer_core::broker::info(&state.tree);
    if !broker.is_empty() {
        heading(ui, "BROKER");
        grid(ui, "broker", broker.into_iter().map(|(key, text)| (key.to_owned(), text)).collect());
    }
    let totals = state.tree.stats();
    heading(ui, "THIS VIEW");
    let mut rows = vec![
        ("Topics".to_owned(), format::count(totals.topics as u64)),
        ("Messages".to_owned(), format::count(totals.messages)),
        ("Rate".to_owned(), format::rate(state.message_rate())),
        ("Stored messages".to_owned(), format::count(totals.stored_messages as u64)),
        ("History memory".to_owned(), format::bytes(totals.stored_bytes)),
    ];
    if let Some(node) = selected.map(|id| state.tree.node(id)) {
        rows.push(("Selected: messages".to_owned(), format::count(node.message_count())));
        rows.push(("Selected: subtopics".to_owned(), format::count(node.subtree_topics() as u64)));
        rows.push(("Selected: messages below".to_owned(), format::count(node.subtree_messages())));
    }
    grid(ui, "stats", rows);
}
