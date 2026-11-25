//! Renders the UI to PNG files for visual inspection. Needs a GPU, so it only runs on request:
//! `cargo test -p mqtt-viewer screenshots -- --ignored`.

use std::path::PathBuf;

use eframe::egui::vec2;
use egui_kittest::Harness;
use mqtt_viewer_core::tree::TopicTree;

use super::{populated, state};
use crate::state::{Action, AppState};

fn render(name: &str, state: AppState) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&directory).unwrap();
    let mut harness = Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        .build_ui_state(|ui, state: &mut AppState| crate::ui::root(ui, state), state);
    harness.run_steps(4);
    harness.render().unwrap().save(directory.join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore = "needs a GPU; run on request to look at the UI"]
fn screenshots() {
    let mut empty = state();
    empty.apply(Action::OpenConnectDialog);
    render("connect", empty);

    let mut browsing = populated();
    browsing.apply(Action::SetExpandedRecursive(TopicTree::ROOT, true));
    let climate = browsing.tree.find("home/kitchen/climate").unwrap();
    browsing.apply(Action::Select(Some(climate)));
    browsing.apply(Action::AddChart { topic: "home/kitchen/climate".into(), pointer: "/temperature".into() });
    render("browse", browsing);

    let mut settings = populated();
    settings.apply(Action::OpenSettings);
    render("settings", settings);
}
