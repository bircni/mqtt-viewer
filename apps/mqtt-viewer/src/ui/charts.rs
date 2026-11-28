//! The chart panel: one card per plot, side by side; a plot can hold several series.

use eframe::egui::{self, Color32, RichText, ScrollArea};
use egui_plot::{HoverPosition, Legend, Line, Plot, PlotPoints};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::fonts::icon;
use super::theme::{self, Palette};
use crate::chart::Chart;
use crate::state::{Action, AppState};

const WINDOWS: [(&str, Option<f64>); 5] =
    [("All", None), ("1 min", Some(60.0)), ("5 min", Some(300.0)), ("1 h", Some(3600.0)), ("24 h", Some(86_400.0))];
/// Narrowest a plot card gets before the panel uses fewer columns.
const MIN_CARD_WIDTH: f32 = 420.0;
const GAP: f32 = 10.0;

/// The colour of the n-th series in a plot.
const fn series_color(palette: &Palette, index: usize) -> Color32 {
    let colors = [palette.accent, palette.ok, palette.warn, palette.key, palette.literal, palette.error];
    colors[index % colors.len()]
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    // Plots in the order their first series was added.
    let mut plots: Vec<(u64, Vec<usize>)> = Vec::new();
    for (index, chart) in state.charts.iter().enumerate() {
        match plots.iter_mut().find(|(plot, _)| *plot == chart.plot) {
            Some((_, members)) => members.push(index),
            None => plots.push((chart.plot, vec![index])),
        }
    }
    ui.horizontal(|ui| {
        ui.label(theme::icon(icon::CHART_LINE, palette.accent));
        ui.label(theme::strong("Charts", 13.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, icon::TRASH, "Remove all charts").clicked() {
                actions.push(Action::ClearCharts);
            }
        });
    });
    ui.add_space(4.0);
    let width = ui.available_width();
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small positive column count")]
    let columns = ((width + GAP) / (MIN_CARD_WIDTH + GAP)).floor().clamp(1.0, 3.0) as usize;
    #[expect(clippy::cast_precision_loss, reason = "a small column count")]
    let card_width = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for row in plots.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for (plot, members) in row {
                    ui.push_id(plot, |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(card_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| plot_card(ui, state, &plots, *plot, members, card_width, actions),
                        );
                    });
                }
            });
            ui.add_space(GAP);
        }
    });
}

fn plot_card(
    ui: &mut egui::Ui,
    state: &mut AppState,
    plots: &[(u64, Vec<usize>)],
    plot: u64,
    members: &[usize],
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = Palette::of(ui);
    theme::card(ui).show(ui, |ui| {
        ui.set_width(width - 28.0);
        let host = members[0];
        series(ui, state, members, actions);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            settings(ui, &mut state.charts, members);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                plot_actions(ui, state, plots, plot, members, actions);
            });
        });
        let chart = &state.charts[host];
        let mut view = Plot::new("plot")
            .height(150.0)
            .allow_scroll(false)
            .x_axis_formatter(|mark, _range| time_label(mark.value))
            .label_formatter(|hover| {
                let (HoverPosition::NearDataPoint { position, .. } | HoverPosition::Elsewhere { position }) = hover;
                Some(format!("{}\n{}", time_label(position.x), position.y))
            });
        if members.len() > 1 {
            view = view.legend(Legend::default());
        }
        if let Some((low, high)) = chart.y_range {
            view = view.default_y_bounds(low, high).auto_bounds([true, false]);
        }
        let (window, stepped) = (chart.window_secs, chart.stepped);
        view.show(ui, |plot_ui| {
            for (position, index) in members.iter().enumerate() {
                let series = &state.charts[*index];
                let shown = Chart { window_secs: window, stepped, ..series.clone() };
                let line = Line::new(series.label(), PlotPoints::from(shown.visible_points()));
                plot_ui.line(line.color(series_color(&palette, position)).width(1.75));
            }
        });
    });
}

/// One line per series: its colour, name and latest value.
fn series(ui: &mut egui::Ui, state: &AppState, members: &[usize], actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    for (position, index) in members.iter().enumerate() {
        let chart = &state.charts[*index];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let color = series_color(&palette, position);
            theme::dot(ui, color, 3.5);
            ui.add(egui::Label::new(theme::strong(chart.label(), 12.5)).truncate());
            if let Some(latest) = chart.latest() {
                ui.label(RichText::new(format!("{latest}")).monospace().color(color));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_only(ui, icon::X, "Remove chart").clicked() {
                    actions.push(Action::RemoveChart(*index));
                }
                if members.len() > 1 && theme::icon_only(ui, icon::EXPORT, "Show separately").clicked() {
                    actions.push(Action::SeparateChart(*index));
                }
            });
        });
    }
}

/// Combining with another plot and the CSV export. Laid out right to left.
fn plot_actions(
    ui: &mut egui::Ui,
    state: &AppState,
    plots: &[(u64, Vec<usize>)],
    plot: u64,
    members: &[usize],
    actions: &mut Vec<Action>,
) {
    let palette = Palette::of(ui);
    if theme::icon_only(ui, icon::FLOPPY_DISK, "Export CSV").clicked()
        && let Some(path) = state.files.save("chart.csv", "CSV", &["csv"])
    {
        actions.push(Action::ExportPlot { plot, path });
    }
    {
        let others: Vec<_> = plots.iter().filter(|(other, _)| *other != plot).collect();
        if !others.is_empty() {
            ui.menu_button((theme::icon(icon::PLUS, palette.text_weak), "Combine"), |ui| {
                for (_, other) in others {
                    let label = state.charts[other[0]].label();
                    if ui.button(format!("Into {label}")).clicked() {
                        for index in members {
                            actions.push(Action::CombineCharts { from: *index, into: other[0] });
                        }
                        ui.close();
                    }
                }
            })
            .response
            .accessible_name("Combine")
            .on_hover_text("Draw these series in another chart");
        }
    }
}

/// The plot's settings, kept on its first series. Pausing applies to every series.
fn settings(ui: &mut egui::Ui, charts: &mut [Chart], members: &[usize]) {
    let host = members[0];
    {
        let mut paused = charts[host].paused;
        if ui.toggle_value(&mut paused, "Pause").changed() {
            for index in members {
                charts[*index].paused = paused;
            }
        }
        let chart = &mut charts[host];
        ui.toggle_value(&mut chart.stepped, "Steps");
        let current = WINDOWS.iter().find(|(_, window)| *window == chart.window_secs).map_or("All", |(label, _)| label);
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
    }
}

fn time_label(unix_seconds: f64) -> String {
    #[expect(clippy::cast_possible_truncation, reason = "plot coordinates are far inside the i64 range")]
    let millis = (unix_seconds * 1000.0) as i64;
    Timestamp::from_millisecond(millis)
        .map_or_else(|_| String::new(), |at| at.to_zoned_datetime(TimeZone::system()).strftime("%H:%M:%S").to_string())
}
