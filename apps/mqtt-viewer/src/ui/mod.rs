//! Drawing code. Everything here reads [`AppState`] and reports input as [`Action`]s; no behaviour lives here.

mod charts;
mod connect;
mod dialogs;
pub(crate) mod fonts;
mod native_picker;
mod sidebar;
mod theme;
mod toolbar;
mod tree;

use eframe::egui::{self, Align2, Key, Margin, Modifiers, Panel, RichText};

pub use self::native_picker::NativePicker;
use crate::state::{Action, AppState, Dialog, Navigate};

/// Draws the whole window.
pub fn root(ui: &mut egui::Ui, state: &mut AppState) {
    theme::apply(ui.ctx(), state.config.settings.theme);
    let mut actions = Vec::new();
    keyboard(ui, state, &mut actions);

    let bar = theme::bar_frame(ui);
    Panel::top("toolbar").frame(bar).show(ui, |ui| toolbar::top(ui, state, &mut actions));
    let status = theme::panel_frame(ui, Margin::symmetric(12, 4));
    Panel::bottom("status").frame(status).show(ui, |ui| toolbar::status(ui, state));
    Panel::right("sidebar")
        .frame(theme::panel_frame(ui, Margin::same(10)))
        .resizable(true)
        .default_size(460.0)
        .size_range(320.0..=900.0)
        .show(ui, |ui| sidebar::show(ui, state, &mut actions));
    if !state.charts.is_empty() {
        Panel::bottom("charts")
            .frame(theme::panel_frame(ui, Margin::symmetric(12, 8)))
            .resizable(true)
            .default_size(290.0)
            .size_range(160.0..=700.0)
            .show(ui, |ui| charts::show(ui, state, &mut actions));
    }
    egui::CentralPanel::default().frame(theme::base_frame(ui)).show(ui, |ui| tree::show(ui, state, &mut actions));

    dialogs::show(ui.ctx(), state, &mut actions);
    notices(ui.ctx(), state, &mut actions);

    for action in actions {
        state.apply(action);
    }
}

fn keyboard(ui: &egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    let ctx = ui.ctx();
    if state.dialog != Dialog::None {
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            actions.push(Action::CloseDialog);
        }
        return;
    }
    // Works from anywhere, also while another field has the focus.
    if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::F)) {
        actions.push(Action::FocusSearch);
    }
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let bindings = [
        (Key::ArrowUp, Navigate::Up),
        (Key::ArrowDown, Navigate::Down),
        (Key::ArrowLeft, Navigate::Left),
        (Key::ArrowRight, Navigate::Right),
        (Key::Home, Navigate::First),
        (Key::End, Navigate::Last),
    ];
    ctx.input(|input| {
        for (key, direction) in bindings {
            if input.key_pressed(key) {
                actions.push(Action::Navigate(direction));
            }
        }
        if input.key_pressed(Key::Space) {
            actions.push(Action::TogglePause);
        }
        if input.key_pressed(Key::Delete) || input.key_pressed(Key::Backspace) {
            actions.push(Action::DeleteSelected);
        }
    });
}

fn notices(ctx: &egui::Context, state: &AppState, actions: &mut Vec<Action>) {
    if state.notices.is_empty() {
        return;
    }
    // Keep repainting so notices disappear on time even without input.
    ctx.request_repaint_after_secs(1.0);
    egui::Area::new(egui::Id::unique("notices")).anchor(Align2::RIGHT_BOTTOM, [-12.0, -36.0]).show(ctx, |ui| {
        ui.set_max_width(420.0);
        let palette = theme::Palette::of(ui);
        for (index, notice) in state.notices.iter().enumerate().rev().take(5) {
            let marker = if notice.is_error { palette.error } else { palette.accent };
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (bar, _) = ui.allocate_exact_size(egui::vec2(3.0, 18.0), egui::Sense::hover());
                    ui.painter().rect_filled(bar, 2.0, marker);
                    let text = RichText::new(&notice.text);
                    ui.add(egui::Label::new(if notice.is_error { text.color(palette.error) } else { text }).wrap());
                    if theme::icon_only(ui, fonts::icon::X, "Dismiss").clicked() {
                        actions.push(Action::DismissNotice(index));
                    }
                });
            });
        }
    });
}
