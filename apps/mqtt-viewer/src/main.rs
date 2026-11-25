//! mqtt-viewer: a native MQTT client that shows broker topics as a live tree.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    mqtt_viewer::run()
}
