//! Modal dialogs: connection profiles, settings, delete confirmation and replay.

use eframe::egui::{self, CollapsingHeader, DragValue, Modal, RichText, ScrollArea, TextEdit};
use mqtt_viewer_core::config::{
    ConnectionProfile, LastWill, ProtoBinding, ProtocolVersion, Settings, Subscription, Theme, TimestampFormat,
    Transport, TreeOrder,
};
use mqtt_viewer_core::message::QoS;
use mqtt_viewer_core::record;

use super::theme::Palette;
use crate::state::{Action, AppState, ConnectDialog, Dialog, ReplayDialog};

/// Height of the connection dialog's two-column body.
const BODY_HEIGHT: f32 = 420.0;

pub fn show(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    // The dialog is taken out while it is drawn so its body can read the rest of the state.
    let mut dialog = std::mem::take(&mut state.dialog);
    match &mut dialog {
        Dialog::None => {}
        Dialog::Connect(connect) => {
            Modal::new(egui::Id::unique("connect")).show(ctx, |ui| connect_dialog(ui, state, connect, actions));
        }
        Dialog::Settings(draft) => {
            Modal::new(egui::Id::unique("settings")).show(ctx, |ui| settings_dialog(ui, state, draft, actions));
        }
        Dialog::ConfirmDelete { topics, .. } => {
            Modal::new(egui::Id::unique("confirm-delete")).show(ctx, |ui| confirm_delete(ui, topics, actions));
        }
        Dialog::Replay(replay) => {
            Modal::new(egui::Id::unique("replay")).show(ctx, |ui| replay_dialog(ui, state, replay, actions));
        }
    }
    state.dialog = dialog;
}

fn connect_dialog(ui: &mut egui::Ui, state: &AppState, dialog: &mut ConnectDialog, actions: &mut Vec<Action>) {
    ui.set_width(760.0);
    ui.heading("MQTT connection");
    ui.separator();
    // A fixed height: a row that sizes itself to its content never settles here, because the vertical
    // separator stretches to the row height and thereby increases it.
    ui.allocate_ui_with_layout(egui::vec2(760.0, BODY_HEIGHT), egui::Layout::left_to_right(egui::Align::Min), |ui| {
        ui.set_height(BODY_HEIGHT);
        ui.vertical(|ui| {
            ui.set_width(200.0);
            ui.strong("Connections");
            ScrollArea::vertical().id_salt("profiles").max_height(BODY_HEIGHT - 60.0).auto_shrink(false).show(
                ui,
                |ui| {
                    for profile in &state.config.profiles {
                        let selected = profile.id == dialog.draft.id;
                        if ui.selectable_label(selected, &profile.name).on_hover_text(profile.url()).clicked() {
                            actions.push(Action::EditProfile(profile.id.clone()));
                        }
                    }
                    if state.config.profiles.is_empty() {
                        ui.weak("No saved connections yet");
                    }
                },
            );
            ui.horizontal(|ui| {
                if ui.button("New").clicked() {
                    actions.push(Action::NewProfile);
                }
                let saved = state.config.profile(&dialog.draft.id).is_some();
                if ui.add_enabled(saved, egui::Button::new("Duplicate")).clicked() {
                    actions.push(Action::DuplicateProfile(dialog.draft.id.clone()));
                }
                if ui.add_enabled(saved, egui::Button::new("Delete")).clicked() {
                    actions.push(Action::DeleteProfile(dialog.draft.id.clone()));
                }
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            ScrollArea::vertical().id_salt("profile").max_height(BODY_HEIGHT).auto_shrink(false).show(ui, |ui| {
                profile_form(ui, &mut dialog.draft, &mut dialog.show_password);
            });
        });
    });
    ui.separator();
    if let Err(problem) = dialog.draft.validate() {
        ui.label(RichText::new(problem).color(Palette::of(ui).error));
    }
    ui.horizontal(|ui| {
        if ui.button(RichText::new("Connect").strong()).clicked() {
            actions.push(Action::Connect);
        }
        if ui.button("Save").clicked() {
            actions.push(Action::SaveProfile);
        }
        if ui.button("Cancel").clicked() {
            actions.push(Action::CloseDialog);
        }
        ui.weak("Passwords are stored unencrypted in the configuration file.");
    });
}

fn profile_form(ui: &mut egui::Ui, draft: &mut ConnectionProfile, show_password: &mut bool) {
    egui::Grid::new("profile-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        ui.add(TextEdit::singleline(&mut draft.name).desired_width(320.0)).accessible_name("Name");
        ui.end_row();

        ui.label("Protocol");
        ui.horizontal(|ui| {
            let before = draft.transport;
            egui::ComboBox::from_id_salt("transport")
                .selected_text(format!("{}://", draft.transport.scheme()))
                .show_ui(ui, |ui| {
                    for transport in Transport::ALL {
                        ui.selectable_value(&mut draft.transport, transport, format!("{}://", transport.scheme()));
                    }
                })
                .response
                .accessible_name("Transport");
            // Follow the transport's usual port unless the user chose a custom one.
            if draft.transport != before && draft.port == before.default_port() {
                draft.port = draft.transport.default_port();
            }
            for version in ProtocolVersion::ALL {
                ui.selectable_value(&mut draft.protocol, version, format!("MQTT {}", version.label()));
            }
        });
        ui.end_row();

        ui.label("Host");
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut draft.host).hint_text("broker.example.org").desired_width(230.0))
                .accessible_name("Host");
            ui.label("Port");
            ui.add(DragValue::new(&mut draft.port).range(1..=65535)).accessible_name("Port");
        });
        ui.end_row();

        if draft.transport.is_websocket() {
            ui.label("Path");
            ui.add(TextEdit::singleline(&mut draft.ws_path).desired_width(320.0)).accessible_name("WebSocket path");
            ui.end_row();
        }

        ui.label("Username");
        ui.add(TextEdit::singleline(&mut draft.username).desired_width(320.0)).accessible_name("Username");
        ui.end_row();

        ui.label("Password");
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut draft.password).password(!*show_password).desired_width(250.0))
                .accessible_name("Password");
            ui.checkbox(show_password, "show");
        });
        ui.end_row();
    });

    session_section(ui, draft);
    subscriptions_section(ui, draft);
    tls_section(ui, draft);
    will_section(ui, draft);
}

fn session_section(ui: &mut egui::Ui, draft: &mut ConnectionProfile) {
    CollapsingHeader::new("Session").show(ui, |ui| {
        egui::Grid::new("session-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Client ID");
            ui.add(
                TextEdit::singleline(&mut draft.client_id).hint_text("generated per connection").desired_width(280.0),
            )
            .accessible_name("Client ID");
            ui.end_row();
            ui.label("Keep alive (s)");
            ui.add(DragValue::new(&mut draft.keep_alive_secs).range(0..=3600)).accessible_name("Keep alive");
            ui.end_row();
            ui.label("Clean session");
            ui.checkbox(&mut draft.clean_session, "").accessible_name("Clean session");
            ui.end_row();
            if draft.protocol == ProtocolVersion::V5 {
                ui.label("Session expiry (s)");
                optional_number(ui, "Session expiry", &mut draft.session_expiry_secs, 3600);
                ui.end_row();
                ui.label("Topic alias maximum");
                optional_number(ui, "Topic alias maximum", &mut draft.topic_alias_max, 16);
                ui.end_row();
            }
        });
    });
}

fn subscriptions_section(ui: &mut egui::Ui, draft: &mut ConnectionProfile) {
    CollapsingHeader::new("Subscriptions").default_open(true).show(ui, |ui| {
        let mut remove = None;
        for (index, subscription) in draft.subscriptions.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut subscription.filter).desired_width(260.0))
                    .accessible_name("Subscription filter");
                qos_selector(ui, &mut subscription.qos);
                if ui.small_button("×").on_hover_text("Remove subscription").clicked() {
                    remove = Some(index);
                }
            })
            .response
            .accessible_name("Transport");
        }
        if let Some(index) = remove {
            draft.subscriptions.remove(index);
        }
        if ui.small_button("Add subscription").clicked() {
            draft.subscriptions.push(Subscription::new("#"));
        }
    });
}

fn tls_section(ui: &mut egui::Ui, draft: &mut ConnectionProfile) {
    if draft.transport.is_tls() {
        CollapsingHeader::new("TLS").default_open(true).show(ui, |ui| {
            ui.checkbox(&mut draft.validate_certificate, "Validate the server certificate");
            if !draft.validate_certificate {
                ui.label(
                    RichText::new("Any certificate is accepted: the connection can be intercepted.")
                        .color(Palette::of(ui).warn),
                );
            }
            file_field(ui, "CA certificate", &mut draft.ca_file);
            file_field(ui, "Client certificate", &mut draft.client_cert_file);
            file_field(ui, "Client key", &mut draft.client_key_file);
        });
    }
}

fn will_section(ui: &mut egui::Ui, draft: &mut ConnectionProfile) {
    CollapsingHeader::new("Last will").show(ui, |ui| {
        let mut enabled = draft.last_will.is_some();
        if ui.checkbox(&mut enabled, "Send a message when the connection is lost").changed() {
            draft.last_will = enabled.then(LastWill::default);
        }
        if let Some(will) = &mut draft.last_will {
            egui::Grid::new("will-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Topic");
                ui.add(TextEdit::singleline(&mut will.topic).desired_width(280.0)).accessible_name("Will topic");
                ui.end_row();
                ui.label("Payload");
                ui.add(TextEdit::singleline(&mut will.payload).desired_width(280.0)).accessible_name("Will payload");
                ui.end_row();
                ui.label("QoS");
                ui.horizontal(|ui| {
                    qos_selector(ui, &mut will.qos);
                    ui.checkbox(&mut will.retain, "retain");
                });
                ui.end_row();
            });
        }
    });
}

fn qos_selector(ui: &mut egui::Ui, qos: &mut QoS) {
    for option in QoS::ALL {
        ui.selectable_value(qos, option, format!("QoS {}", option.level()));
    }
}

fn optional_number<N: egui::emath::Numeric>(ui: &mut egui::Ui, name: &str, value: &mut Option<N>, default: N) {
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, "").accessible_name(format!("Enable {name}")).changed() {
            *value = enabled.then_some(default);
        }
        if let Some(number) = value {
            ui.add(DragValue::new(number)).accessible_name(name);
        }
    });
}

fn file_field(ui: &mut egui::Ui, label: &str, file: &mut Option<std::path::PathBuf>) {
    ui.horizontal(|ui| {
        ui.label(label);
        match file {
            Some(path) => {
                ui.monospace(path.display().to_string());
                if ui.small_button("×").clicked() {
                    *file = None;
                }
            }
            None => {
                ui.weak("none");
            }
        }
        if ui.small_button("Choose…").clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("PEM", &["pem", "crt", "cer", "key"]).pick_file()
        {
            *file = Some(path);
        }
    });
}

fn settings_dialog(ui: &mut egui::Ui, state: &AppState, draft: &mut Settings, actions: &mut Vec<Action>) {
    ui.set_width(560.0);
    ui.heading("Settings");
    ui.separator();
    ScrollArea::vertical().max_height(440.0).auto_shrink(false).show(ui, |ui| {
        egui::Grid::new("settings-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Theme");
            ui.horizontal(|ui| {
                for (theme, label) in [(Theme::System, "System"), (Theme::Light, "Light"), (Theme::Dark, "Dark")] {
                    ui.selectable_value(&mut draft.theme, theme, label);
                }
            });
            ui.end_row();

            ui.label("Topic order");
            ui.horizontal(|ui| {
                let orders = [
                    (TreeOrder::Alphabetical, "A–Z"),
                    (TreeOrder::Messages, "Most messages"),
                    (TreeOrder::Topics, "Most topics"),
                ];
                for (order, label) in orders {
                    ui.selectable_value(&mut draft.tree_order, order, label);
                }
            });
            ui.end_row();

            ui.label("Activity");
            ui.checkbox(&mut draft.highlight_activity, "Highlight topics when a message arrives");
            ui.end_row();

            ui.label("Timestamps");
            ui.horizontal(|ui| {
                let formats = [
                    (TimestampFormat::Time, "Time"),
                    (TimestampFormat::DateTime, "Date and time"),
                    (TimestampFormat::Iso, "ISO 8601"),
                    (TimestampFormat::Relative, "Relative"),
                ];
                for (format, label) in formats {
                    ui.selectable_value(&mut draft.timestamp_format, format, label);
                }
            });
            ui.end_row();

            ui.label("History per topic");
            ui.add(DragValue::new(&mut draft.history_cap).range(1..=100_000).suffix(" messages"))
                .accessible_name("History per topic");
            ui.end_row();

            ui.label("Memory budget");
            ui.add(DragValue::new(&mut draft.memory_budget_mb).range(8..=65_536).suffix(" MiB"))
                .accessible_name("Memory budget");
            ui.end_row();
        });

        ui.separator();
        ui.strong("History on disk");
        ui.checkbox(&mut draft.persist_history, "Store received messages in a local database");
        ui.add_enabled_ui(draft.persist_history, |ui| {
            ui.horizontal(|ui| {
                ui.label("Keep for");
                ui.add(DragValue::new(&mut draft.retention_days).range(0..=3650).suffix(" days"))
                    .accessible_name("Retention days");
                ui.label("and at most");
                ui.add(DragValue::new(&mut draft.retention_mb).range(0..=1_048_576).suffix(" MiB"))
                    .accessible_name("Retention size");
            });
            ui.weak("0 disables the limit.");
        });
        if ui.button("Clear stored history").clicked() {
            actions.push(Action::ClearStoredHistory);
        }

        ui.separator();
        ui.strong("Protobuf");
        protobuf_settings(ui, state, actions);
    });
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(RichText::new("Save").strong()).clicked() {
            actions.push(Action::ApplySettings);
        }
        if ui.button("Cancel").clicked() {
            actions.push(Action::CloseDialog);
        }
    });
}

fn protobuf_settings(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    for (index, file) in state.config.proto_files.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.monospace(file.display().to_string());
            if ui.small_button("×").on_hover_text("Remove schema").clicked() {
                actions.push(Action::RemoveProtoFile(index));
            }
        });
    }
    if ui.button("Add .proto file…").clicked()
        && let Some(path) = rfd::FileDialog::new().add_filter("Protobuf", &["proto"]).pick_file()
    {
        actions.push(Action::AddProtoFile(path));
    }
    ui.label("Topic bindings");
    for (index, binding) in state.config.proto_bindings.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.monospace(&binding.pattern);
            ui.weak("as");
            ui.monospace(&binding.message_type);
            if ui.small_button("×").on_hover_text("Remove binding").clicked() {
                actions.push(Action::RemoveProtoBinding(index));
            }
        });
    }
    let id = ui.scope_id().with("new-binding");
    let mut binding = ui.data_mut(|data| data.get_temp::<(String, String)>(id).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add(TextEdit::singleline(&mut binding.0).hint_text("devices/+/telemetry").desired_width(180.0))
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
    ui.set_width(460.0);
    ui.heading("Delete retained messages");
    ui.label(format!("This clears the retained message of {} topics on the broker:", topics.len()));
    ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        for topic in topics.iter().take(200) {
            ui.monospace(topic);
        }
        if topics.len() > 200 {
            ui.weak(format!("… and {} more", topics.len() - 200));
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(RichText::new("Delete").color(Palette::of(ui).error)).clicked() {
            actions.push(Action::ConfirmDelete);
        }
        if ui.button("Cancel").clicked() {
            actions.push(Action::CloseDialog);
        }
    });
}

fn replay_dialog(ui: &mut egui::Ui, state: &AppState, dialog: &mut ReplayDialog, actions: &mut Vec<Action>) {
    ui.set_width(480.0);
    ui.heading("Replay recording");
    ui.monospace(dialog.path.display().to_string());
    let steps = record::plan_replay(&dialog.messages, &dialog.options);
    ui.label(format!(
        "{} messages, {:.1} s at the chosen speed",
        dialog.messages.len(),
        record::replay_duration(&steps).as_secs_f64()
    ));
    ui.separator();
    egui::Grid::new("replay-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Speed");
        ui.horizontal(|ui| {
            ui.add(DragValue::new(&mut dialog.options.speed).range(0.0..=1000.0).speed(0.1).suffix("×"))
                .accessible_name("Replay speed");
            ui.weak("0 sends everything at once");
        });
        ui.end_row();
        ui.label("Replace topic prefix");
        ui.add(TextEdit::singleline(&mut dialog.options.prefix_from).hint_text("home/").desired_width(220.0))
            .accessible_name("Prefix to replace");
        ui.end_row();
        ui.label("with");
        ui.add(TextEdit::singleline(&mut dialog.options.prefix_to).hint_text("sandbox/home/").desired_width(220.0))
            .accessible_name("Replacement prefix");
        ui.end_row();
    });
    ui.separator();
    let target = state.profile.as_ref().filter(|_| state.connection.is_active()).map(ConnectionProfile::url);
    match &target {
        Some(url) => {
            ui.label(RichText::new(format!("The messages will be published to {url}")).color(Palette::of(ui).warn));
        }
        None => {
            ui.label(RichText::new("Not connected to a broker").color(Palette::of(ui).error));
        }
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(target.is_some(), egui::Button::new(RichText::new("Start replay").strong())).clicked() {
            actions.push(Action::StartReplay);
        }
        if ui.button("Cancel").clicked() {
            actions.push(Action::CloseDialog);
        }
    });
}
