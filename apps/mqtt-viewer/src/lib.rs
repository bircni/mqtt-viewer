//! The mqtt-viewer desktop application.
//!
//! [`state`] holds everything the app knows and all of its behaviour; [`ui`] only draws that state and turns
//! input into [`state::Action`]s. [`app`] glues both to the window and the broker connection.

pub mod app;
pub mod chart;
pub mod format;
pub mod forms;
pub mod highlight;
pub mod icon;
pub mod picker;
pub mod state;
pub mod ui;

#[cfg(test)]
mod tests;

const ICON_SIZE: u32 = 256;

/// Opens the main window and runs until it is closed.
///
/// # Errors
/// Fails when the window or the graphics context cannot be created.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MQTT Viewer")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(eframe::egui::IconData { rgba: window_icon(), width: ICON_SIZE, height: ICON_SIZE }),
        ..Default::default()
    };
    eframe::run_native("mqtt-viewer", options, Box::new(|creation| Ok(Box::new(app::App::new(creation)?))))
}

/// The icon for the window, the taskbar and the Dock. macOS shows it in the Dock, where icons keep a margin.
fn window_icon() -> Vec<u8> {
    icon::rgba_inset(ICON_SIZE, if cfg!(target_os = "macos") { icon::MACOS_INSET } else { 0.0 })
}
