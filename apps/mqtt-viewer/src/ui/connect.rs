//! The connection dialog: saved profiles on the left, the selected one's settings in tabs on the right.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{
    self, Align, CornerRadius, DragValue, FontId, Frame, Galley, Layout, Margin, Rect, RichText, Role, ScrollArea,
    Sense, Stroke, TextEdit, WidgetInfo, pos2, vec2,
};
use mqtt_viewer_core::config::{ConnectionProfile, LastWill, ProtocolVersion, Subscription, Transport};
use mqtt_viewer_core::message::QoS;

use super::fonts::{self, icon};
use super::theme::{self, Palette};
use crate::picker::FilePicker;
use crate::state::{Action, AppState, ConnectDialog};

const WIDTH: f32 = 880.0;
const RAIL_WIDTH: f32 = 236.0;
/// Height of the rail and the form; fixed so the dialog does not jump between tabs.
const BODY_HEIGHT: f32 = 500.0;
const GAP: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    General,
    Session,
    Subscriptions,
    Tls,
    Will,
}

pub fn show(ui: &mut egui::Ui, state: &AppState, dialog: &mut ConnectDialog, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.set_width(WIDTH);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ui.allocate_ui_with_layout(vec2(WIDTH, BODY_HEIGHT), Layout::left_to_right(Align::Min), |ui| {
        Frame::new()
            .fill(palette.panel)
            .corner_radius(CornerRadius { nw: 14, ne: 0, sw: 0, se: 0 })
            .inner_margin(Margin::symmetric(12, 16))
            .show(ui, |ui| {
                ui.set_width(RAIL_WIDTH - 24.0);
                ui.set_height(BODY_HEIGHT - 32.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    rail(ui, state, dialog, actions);
                });
            });
        let line = ui.cursor().left();
        ui.painter().vline(line, ui.max_rect().y_range(), Stroke::new(1.0, palette.border));
        Frame::new().inner_margin(Margin { left: 28, right: 28, top: 22, bottom: 12 }).show(ui, |ui| {
            ui.set_width(WIDTH - RAIL_WIDTH - 57.0);
            ui.set_height(BODY_HEIGHT - 34.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                form(ui, state.files.as_ref(), dialog);
            });
        });
    });
    footer(ui, dialog, actions);
}

fn rail(ui: &mut egui::Ui, state: &AppState, dialog: &ConnectDialog, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new("CONNECTIONS").font(fonts::semibold(11.0)).color(palette.text_weak));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_only(ui, icon::PLUS, "New").on_hover_text("New connection").clicked() {
                actions.push(Action::NewProfile);
            }
        });
    });
    let saved = state.config.profile(&dialog.draft.id).is_some();
    ScrollArea::vertical().id_salt("profiles").max_height(BODY_HEIGHT - 120.0).auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for profile in &state.config.profiles {
            let selected = profile.id == dialog.draft.id;
            let row = profile_row(ui, profile, selected).on_hover_text("Double-click to connect");
            if row.double_clicked() {
                actions.push(Action::ConnectProfile(profile.id.clone()));
            } else if row.clicked() {
                actions.push(Action::EditProfile(profile.id.clone()));
            }
        }
        if state.config.profiles.is_empty() {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No saved connections yet").small().color(palette.text_weak));
            });
        }
    });
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let duplicate = ui.add_enabled_ui(saved, |ui| theme::icon_button(ui, icon::COPY, "Duplicate")).inner;
            if duplicate.clicked() {
                actions.push(Action::DuplicateProfile(dialog.draft.id.clone()));
            }
            let delete = ui.add_enabled_ui(saved, |ui| theme::icon_button(ui, icon::TRASH, "Delete")).inner;
            if delete.clicked() {
                actions.push(Action::DeleteProfile(dialog.draft.id.clone()));
            }
        });
    });
}

/// One saved connection: its name above its URL.
fn profile_row(ui: &mut egui::Ui, profile: &ConnectionProfile, selected: bool) -> egui::Response {
    let palette = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &profile.name));
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 8.0, palette.selection);
        let marker = Rect::from_min_size(pos2(rect.left(), rect.top() + 12.0), vec2(3.0, rect.height() - 24.0));
        painter.rect_filled(marker, 1.5, palette.accent);
    } else if response.hovered() {
        painter.rect_filled(rect, 8.0, palette.control);
    }
    let width = rect.width() - 28.0;
    let name = if profile.name.is_empty() { "Unnamed" } else { profile.name.as_str() };
    let title = one_line(ui, name, fonts::semibold(13.0), palette.text_strong, width);
    let url = one_line(ui, &profile.url(), FontId::proportional(11.5), palette.text_weak, width);
    painter.galley(pos2(rect.left() + 14.0, rect.top() + 7.0), title, palette.text_strong);
    painter.galley(pos2(rect.left() + 14.0, rect.top() + 27.0), url, palette.text_weak);
    response
}

fn one_line(ui: &egui::Ui, text: &str, font_id: FontId, color: egui::Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat { font_id, color, ..Default::default() });
    job.wrap = TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, ..Default::default() };
    ui.painter().layout_job(job)
}

fn form(ui: &mut egui::Ui, files: &dyn FilePicker, dialog: &mut ConnectDialog) {
    let palette = Palette::of(ui);
    let draft = &mut dialog.draft;
    ui.horizontal(|ui| {
        let glyph = if draft.transport.is_tls() { icon::SHIELD_CHECK } else { icon::BROADCAST };
        Frame::new().fill(palette.selection).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(9)).show(
            ui,
            |ui| {
                ui.label(RichText::new(glyph).size(20.0).color(palette.accent));
            },
        );
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(theme::strong("MQTT connection", 17.0).color(palette.text_strong));
            ui.label(RichText::new(draft.url()).monospace().color(palette.text_weak));
        });
    });
    ui.add_space(10.0);

    let id = ui.make_persistent_id("connect-tab");
    let mut tab = ui.data(|data| data.get_temp::<Tab>(id)).unwrap_or_default();
    if tab == Tab::Tls && !draft.transport.is_tls() {
        tab = Tab::General;
    }
    let mut options = vec![
        (Tab::General, icon::GEAR_SIX, "General"),
        (Tab::Session, icon::CLOCK, "Session"),
        (Tab::Subscriptions, icon::HASH, "Subscriptions"),
    ];
    if draft.transport.is_tls() {
        options.push((Tab::Tls, icon::SHIELD_CHECK, "TLS"));
    }
    options.push((Tab::Will, icon::PAPER_PLANE_TILT, "Last will"));
    if let Some(chosen) = theme::tabs(ui, tab, &options) {
        tab = chosen;
    }
    ui.data_mut(|data| data.insert_temp(id, tab));

    ScrollArea::vertical().id_salt(("profile", tab as u8)).auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 14.0;
        match tab {
            Tab::General => general(ui, draft, &mut dialog.show_password),
            Tab::Session => session(ui, draft),
            Tab::Subscriptions => subscriptions(ui, draft),
            Tab::Tls => tls(ui, files, draft),
            Tab::Will => will(ui, draft),
        }
    });
}

/// A single-line text input of the given width with the dialog's padding.
fn text(value: &mut String, width: f32) -> TextEdit<'_> {
    TextEdit::singleline(value).desired_width(width).margin(vec2(9.0, 6.0))
}

fn general(ui: &mut egui::Ui, draft: &mut ConnectionProfile, show_password: &mut bool) {
    let width = ui.available_width();
    let half = (width - GAP) / 2.0;
    theme::field(ui, "Name", |ui| ui.add(text(&mut draft.name, width)).accessible_name("Name"));

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        theme::field(ui, "Transport", |ui| {
            let before = draft.transport;
            egui::ComboBox::from_id_salt("transport")
                .selected_text(format!("{}://", draft.transport.scheme()))
                .width(half)
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
        });
        theme::field(ui, "Protocol", |ui| {
            let versions = ProtocolVersion::ALL.map(|version| (version, version_label(version)));
            theme::segmented_value(ui, &mut draft.protocol, &versions);
        });
    });

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let port_width = 110.0;
        theme::field(ui, "Host", |ui| {
            ui.add(text(&mut draft.host, width - port_width - GAP).hint_text("broker.example.org"))
                .accessible_name("Host")
        });
        theme::field(ui, "Port", |ui| {
            ui.add_sized(vec2(port_width, 30.0), DragValue::new(&mut draft.port).range(1..=65535))
                .accessible_name("Port")
        });
    });

    if draft.transport.is_websocket() {
        theme::field(ui, "Path", |ui| ui.add(text(&mut draft.ws_path, width)).accessible_name("WebSocket path"));
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        theme::field(ui, "Username", |ui| {
            ui.add(text(&mut draft.username, half).hint_text("optional")).accessible_name("Username")
        });
        theme::field(ui, "Password", |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(text(&mut draft.password, half - 70.0).password(!*show_password)).accessible_name("Password");
                ui.toggle_value(show_password, "show");
            });
        });
    });
}

const fn version_label(version: ProtocolVersion) -> &'static str {
    match version {
        ProtocolVersion::V311 => "MQTT 3.1.1",
        ProtocolVersion::V5 => "MQTT 5.0",
    }
}

fn session(ui: &mut egui::Ui, draft: &mut ConnectionProfile) {
    let width = ui.available_width();
    let half = (width - GAP) / 2.0;
    theme::field(ui, "Client ID", |ui| {
        ui.add(text(&mut draft.client_id, width).hint_text("generated per connection")).accessible_name("Client ID")
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        theme::field(ui, "Keep alive (s)", |ui| {
            ui.add_sized(vec2(half, 30.0), DragValue::new(&mut draft.keep_alive_secs).range(0..=3600))
                .accessible_name("Keep alive")
        });
        theme::field(ui, "Clean session", |ui| {
            ui.checkbox(&mut draft.clean_session, "Start without stored state").accessible_name("Clean session")
        });
    });
    if draft.protocol == ProtocolVersion::V5 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            theme::field(ui, "Session expiry (s)", |ui| {
                optional_number(ui, "Session expiry", &mut draft.session_expiry_secs, 3600, half);
            });
            theme::field(ui, "Topic alias maximum", |ui| {
                optional_number(ui, "Topic alias maximum", &mut draft.topic_alias_max, 16, half);
            });
        });
    } else {
        hint(ui, icon::INFO, "Session expiry and topic aliases are available with MQTT 5.0.");
    }
}

fn subscriptions(ui: &mut egui::Ui, draft: &mut ConnectionProfile) {
    let palette = Palette::of(ui);
    ui.spacing_mut().item_spacing.y = 8.0;
    let mut remove = None;
    let filter_width = ui.available_width() - 170.0;
    for (index, subscription) in draft.subscriptions.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(text(&mut subscription.filter, filter_width).font(egui::TextStyle::Monospace))
                .accessible_name("Subscription filter");
            qos_selector(ui, &mut subscription.qos);
            if theme::icon_only(ui, icon::X, "Remove subscription").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        draft.subscriptions.remove(index);
    }
    if draft.subscriptions.is_empty() {
        ui.label(RichText::new("Without a subscription no topics arrive.").color(palette.warn));
    }
    if theme::icon_button(ui, icon::PLUS, "Add subscription").clicked() {
        draft.subscriptions.push(Subscription::new("#"));
    }
    hint(ui, icon::INFO, "Use + for one level and # for everything below, as in home/+/temperature.");
}

fn tls(ui: &mut egui::Ui, files: &dyn FilePicker, draft: &mut ConnectionProfile) {
    ui.checkbox(&mut draft.validate_certificate, "Validate the server certificate");
    if !draft.validate_certificate {
        hint(ui, icon::WARNING, "Any certificate is accepted: the connection can be intercepted.");
    }
    file_field(ui, files, "CA certificate", &mut draft.ca_file);
    file_field(ui, files, "Client certificate", &mut draft.client_cert_file);
    file_field(ui, files, "Client key", &mut draft.client_key_file);
}

fn will(ui: &mut egui::Ui, draft: &mut ConnectionProfile) {
    let width = ui.available_width();
    let mut enabled = draft.last_will.is_some();
    if ui.checkbox(&mut enabled, "Send a message when the connection is lost").changed() {
        draft.last_will = enabled.then(LastWill::default);
    }
    if let Some(will) = &mut draft.last_will {
        theme::field(ui, "Topic", |ui| ui.add(text(&mut will.topic, width)).accessible_name("Will topic"));
        theme::field(ui, "Payload", |ui| ui.add(text(&mut will.payload, width)).accessible_name("Will payload"));
        theme::field(ui, "QoS", |ui| {
            ui.horizontal(|ui| {
                qos_selector(ui, &mut will.qos);
                ui.checkbox(&mut will.retain, "retain");
            });
        });
    }
}

fn qos_selector(ui: &mut egui::Ui, qos: &mut QoS) {
    let labels = ["QoS 0", "QoS 1", "QoS 2"];
    let options = QoS::ALL.map(|option| (option, labels[usize::from(option.level())]));
    theme::segmented_value(ui, qos, &options);
}

fn optional_number<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    name: &str,
    value: &mut Option<N>,
    default: N,
    width: f32,
) {
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, "").accessible_name(format!("Enable {name}")).changed() {
            *value = enabled.then_some(default);
        }
        if let Some(number) = value {
            ui.add_sized(vec2(width - 32.0, 30.0), DragValue::new(number)).accessible_name(name);
        } else {
            ui.weak("not sent");
        }
    });
}

fn file_field(ui: &mut egui::Ui, files: &dyn FilePicker, label: &str, file: &mut Option<PathBuf>) {
    let palette = Palette::of(ui);
    theme::field(ui, label, |ui| {
        Frame::new()
            .fill(palette.sunken)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin { left: 10, right: 4, top: 3, bottom: 3 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    match file {
                        Some(path) => {
                            ui.label(RichText::new(path.display().to_string()).monospace().color(palette.text));
                        }
                        None => {
                            ui.label(RichText::new("none").color(palette.text_weak));
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let choose = theme::icon_button(ui, icon::FOLDER_OPEN, "Choose…")
                            .accessible_name(format!("Choose {label}"));
                        if choose.clicked()
                            && let Some(path) = files.open("PEM", &["pem", "crt", "cer", "key"])
                        {
                            *file = Some(path);
                        }
                        if file.is_some() && theme::icon_only(ui, icon::X, "Clear").clicked() {
                            *file = None;
                        }
                    });
                });
            });
    });
}

/// A short explanation in a tinted box.
fn hint(ui: &mut egui::Ui, glyph: &str, text: &str) -> egui::Response {
    let palette = Palette::of(ui);
    let color = if glyph == icon::WARNING { palette.warn } else { palette.text_weak };
    Frame::new()
        .fill(color.gamma_multiply(0.1))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(theme::icon(glyph, color));
                ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
            });
        })
        .response
}

fn footer(ui: &mut egui::Ui, dialog: &ConnectDialog, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    let top = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, palette.border));
    Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
        ui.set_width(WIDTH - 40.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Err(problem) = dialog.draft.validate() {
                ui.label(theme::icon(icon::WARNING, palette.error));
                ui.label(RichText::new(problem).color(palette.error));
            } else {
                ui.label(theme::icon(icon::LOCK_SIMPLE, palette.text_weak));
                ui.label(
                    RichText::new("Passwords are stored unencrypted in the configuration file.")
                        .small()
                        .color(palette.text_weak),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let connect = theme::primary_button(ui, Some(icon::PLUGS_CONNECTED), "Connect");
                if theme::add_filled(ui, connect, "Connect").clicked() {
                    actions.push(Action::Connect);
                }
                if theme::icon_button(ui, icon::FLOPPY_DISK, "Save").clicked() {
                    actions.push(Action::SaveProfile);
                }
                if ui.add(egui::Button::new("Cancel").frame_when_inactive(false)).clicked() {
                    actions.push(Action::CloseDialog);
                }
            });
        });
    });
}
