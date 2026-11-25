//! The chart panel.

use eframe::egui::{self, ScrollArea};
use egui_plot::{HoverPosition, Line, Plot, PlotPoints};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::state::{Action, AppState};

const WINDOWS: [(&str, Option<f64>); 5] =
    [("All", None), ("1 min", Some(60.0)), ("5 min", Some(300.0)), ("1 h", Some(3600.0)), ("24 h", Some(86_400.0))];

pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for (index, chart) in state.charts.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(chart.label());
                    if let Some(latest) = chart.latest() {
                        ui.label(format!("{latest}"));
                    }
                    ui.toggle_value(&mut chart.paused, "Pause");
                    ui.toggle_value(&mut chart.stepped, "Steps");
                    let current = WINDOWS
                        .iter()
                        .find(|(_, window)| *window == chart.window_secs)
                        .map_or("All", |(label, _)| label);
                    egui::ComboBox::from_id_salt("window")
                        .selected_text(current)
                        .width(70.0)
                        .show_ui(ui, |ui| {
                            for (label, window) in WINDOWS {
                                ui.selectable_value(&mut chart.window_secs, window, label);
                            }
                        })
                        .response
                        .accessible_name("Time window");
                    let mut fixed = chart.y_range.is_some();
                    if ui.checkbox(&mut fixed, "Fixed range").changed() {
                        chart.y_range = fixed.then(|| {
                            let values = chart.points.iter().map(|point| point[1]);
                            let low = values.clone().fold(f64::INFINITY, f64::min);
                            let high = values.fold(f64::NEG_INFINITY, f64::max);
                            if low.is_finite() && high.is_finite() { (low, high) } else { (0.0, 1.0) }
                        });
                    }
                    if let Some((low, high)) = &mut chart.y_range {
                        ui.add(egui::DragValue::new(low).speed(0.1).prefix("min ")).accessible_name("Range minimum");
                        ui.add(egui::DragValue::new(high).speed(0.1).prefix("max ")).accessible_name("Range maximum");
                    }
                    if ui.small_button("×").on_hover_text("Remove chart").clicked() {
                        actions.push(Action::RemoveChart(index));
                    }
                });
                let points = chart.visible_points();
                let mut plot = Plot::new("plot")
                    .height(150.0)
                    .allow_scroll(false)
                    .x_axis_formatter(|mark, _range| time_label(mark.value))
                    .label_formatter(|hover| {
                        let (HoverPosition::NearDataPoint { position, .. } | HoverPosition::Elsewhere { position }) =
                            hover;
                        Some(format!("{}\n{}", time_label(position.x), position.y))
                    });
                if let Some((low, high)) = chart.y_range {
                    plot = plot.default_y_bounds(low, high).auto_bounds([true, false]);
                }
                plot.show(ui, |plot_ui| plot_ui.line(Line::new(chart.label(), PlotPoints::from(points))));
            });
            ui.separator();
        }
    });
}

fn time_label(unix_seconds: f64) -> String {
    #[expect(clippy::cast_possible_truncation, reason = "plot coordinates are far inside the i64 range")]
    let millis = (unix_seconds * 1000.0) as i64;
    Timestamp::from_millisecond(millis)
        .map_or_else(|_| String::new(), |at| at.to_zoned_datetime(TimeZone::system()).strftime("%H:%M:%S").to_string())
}
