//! Colours and theme handling.

use eframe::egui::{self, Color32, ThemePreference};
use mqtt_viewer_core::config::Theme;

/// Colours that depend on whether the dark or the light theme is active.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Flash colour for rows that just received a message.
    pub activity: Color32,
    /// Background of the selected row.
    pub selection: Color32,
    /// Added lines in a diff.
    pub added: Color32,
    /// Removed lines in a diff.
    pub removed: Color32,
    /// JSON keys.
    pub key: Color32,
    /// JSON strings.
    pub string: Color32,
    /// JSON numbers.
    pub number: Color32,
    /// JSON literals.
    pub literal: Color32,
    /// Connected indicator.
    pub ok: Color32,
    /// Reconnecting indicator.
    pub warn: Color32,
    /// Error text and the disconnected indicator.
    pub error: Color32,
}

impl Palette {
    /// The palette matching the UI's current visuals.
    pub fn of(ui: &egui::Ui) -> Self {
        if ui.visuals().dark_mode {
            Self {
                activity: Color32::from_rgb(64, 120, 72),
                selection: Color32::from_rgb(38, 79, 120),
                added: Color32::from_rgb(120, 200, 130),
                removed: Color32::from_rgb(235, 120, 120),
                key: Color32::from_rgb(130, 180, 250),
                string: Color32::from_rgb(190, 220, 140),
                number: Color32::from_rgb(240, 180, 110),
                literal: Color32::from_rgb(210, 150, 240),
                ok: Color32::from_rgb(90, 200, 110),
                warn: Color32::from_rgb(240, 180, 60),
                error: Color32::from_rgb(240, 100, 100),
            }
        } else {
            Self {
                activity: Color32::from_rgb(190, 235, 195),
                selection: Color32::from_rgb(200, 222, 250),
                added: Color32::from_rgb(20, 120, 40),
                removed: Color32::from_rgb(180, 30, 30),
                key: Color32::from_rgb(20, 80, 170),
                string: Color32::from_rgb(40, 120, 30),
                number: Color32::from_rgb(170, 90, 0),
                literal: Color32::from_rgb(130, 40, 170),
                ok: Color32::from_rgb(30, 150, 60),
                warn: Color32::from_rgb(190, 130, 0),
                error: Color32::from_rgb(200, 40, 40),
            }
        }
    }
}

/// Applies the configured theme.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let preference = match theme {
        Theme::System => ThemePreference::System,
        Theme::Light => ThemePreference::Light,
        Theme::Dark => ThemePreference::Dark,
    };
    if ctx.options(|options| options.theme_preference) != preference {
        ctx.set_theme(preference);
    }
}
