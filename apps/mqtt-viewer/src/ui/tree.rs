//! The topic tree. Only the rows inside the viewport are laid out, so the cost per frame does not depend on
//! the number of topics.

use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{self, Align, FontId, Rect, Role, ScrollArea, Sense, Stroke, TextStyle, WidgetInfo, pos2, vec2};
use jiff::Timestamp;
use mqtt_viewer_core::decode;
use mqtt_viewer_core::tree::{NodeId, Row};

use super::theme::Palette;
use crate::format;
use crate::state::{Action, AppState};

const INDENT: f32 = 16.0;
const ARROW_WIDTH: f32 = 18.0;
/// Seconds a row stays tinted after a message.
const FLASH_SECS: f64 = 1.2;
const PREVIEW_CHARS: usize = 120;

pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let total = state.tree.rows().len();
    if total == 0 {
        ui.centered_and_justified(|ui| {
            let hint = if state.tree.stats().topics > 0 {
                "No topic matches the search"
            } else if state.connection.is_active() {
                "Waiting for messages…"
            } else {
                "Connect to a broker to see its topics"
            };
            ui.weak(hint);
        });
        return;
    }
    let reveal = std::mem::take(&mut state.reveal_selection)
        .then(|| state.selected.and_then(|id| state.tree.cached_rows().iter().position(|row| row.id == id)))
        .flatten();
    let state = &*state;
    let palette = Palette::of(ui);
    let font = TextStyle::Body.resolve(ui.style());
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font)) + 6.0;
    let now = Timestamp::now();
    let mut flashing = false;

    ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, viewport| {
        #[expect(clippy::cast_precision_loss, reason = "row counts are far below 2^24")]
        ui.set_height(row_height * total as f32);
        let origin = ui.max_rect().min;
        let width = ui.available_width();
        let row_rect = |index: usize| {
            #[expect(clippy::cast_precision_loss, reason = "row counts are far below 2^24")]
            let top = origin.y + row_height * index as f32;
            Rect::from_min_size(pos2(origin.x, top), vec2(width, row_height))
        };
        if let Some(index) = reveal {
            ui.scroll_to_rect(row_rect(index), None);
        }
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the row count")]
        let first = ((viewport.min.y / row_height).floor().max(0.0) as usize).min(total);
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the row count")]
        let last = ((viewport.max.y / row_height).ceil().max(0.0) as usize).min(total);
        for index in first..last {
            let row = state.tree.cached_rows()[index];
            flashing |= draw_row(ui, state, row, row_rect(index), &font, &palette, now, actions);
        }
    });
    if flashing {
        ui.ctx().request_repaint_after_secs(0.05);
    }
}

/// Draws one row and returns whether it is still fading out its activity highlight.
#[expect(clippy::too_many_arguments, reason = "per-frame values shared by all rows")]
fn draw_row(
    ui: &egui::Ui,
    state: &AppState,
    row: Row,
    rect: Rect,
    font: &FontId,
    palette: &Palette,
    now: Timestamp,
    actions: &mut Vec<Action>,
) -> bool {
    let node = state.tree.node(row.id);
    let response = ui.interact(rect, ui.scope_id().with(row.id), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, node.name()));
    let selected = state.selected == Some(row.id);

    let age = state
        .activity_age(row.id, now)
        .filter(|age| state.config.settings.highlight_activity && (0.0..FLASH_SECS).contains(age));
    if selected {
        ui.painter().rect_filled(rect, 3.0, palette.selection);
    } else if let Some(age) = age {
        #[expect(clippy::cast_possible_truncation, reason = "a fraction between 0 and 1")]
        let strength = (1.0 - age / FLASH_SECS).clamp(0.0, 1.0) as f32;
        ui.painter().rect_filled(rect, 3.0, palette.activity.gamma_multiply(strength));
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }

    #[expect(clippy::cast_precision_loss, reason = "topic depth is small")]
    let left = rect.left() + 4.0 + INDENT * row.depth as f32;
    let arrow_rect = Rect::from_min_size(pos2(left, rect.top()), vec2(ARROW_WIDTH, rect.height()));
    if row.has_children {
        // Drawn as a shape: arrow glyphs are not in the built-in font.
        let center = arrow_rect.center();
        let half = font.size * 0.28;
        let points = if row.expanded {
            vec![
                pos2(center.x - half, center.y - half * 0.6),
                pos2(center.x + half, center.y - half * 0.6),
                pos2(center.x, center.y + half * 0.8),
            ]
        } else {
            vec![
                pos2(center.x - half * 0.6, center.y - half),
                pos2(center.x - half * 0.6, center.y + half),
                pos2(center.x + half * 0.8, center.y),
            ]
        };
        ui.painter().add(egui::Shape::convex_polygon(points, ui.visuals().weak_text_color(), Stroke::NONE));
    }

    let text_color = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let mut job = LayoutJob::default();
    let name = if node.name().is_empty() { "(empty)" } else { node.name() };
    job.append(name, 0.0, TextFormat { font_id: font.clone(), color: text_color, ..Default::default() });
    if let Some(entry) = node.last() {
        let preview = decode::preview(&entry.payload, PREVIEW_CHARS);
        job.append(" = ", 0.0, TextFormat { font_id: font.clone(), color: weak, ..Default::default() });
        job.append(
            &preview,
            0.0,
            TextFormat { font_id: font.clone(), color: text_color.gamma_multiply(0.85), ..Default::default() },
        );
    }
    if row.has_children {
        let summary = format!(
            "  ({}, {})",
            format::plural(node.subtree_topics() as u64, "topic"),
            format::plural(node.subtree_messages(), "message")
        );
        job.append(&summary, 0.0, TextFormat { font_id: font.clone(), color: weak, ..Default::default() });
    }
    let text_left = arrow_rect.right();
    job.wrap = TextWrapping {
        max_width: (rect.right() - text_left - 4.0).max(0.0),
        max_rows: 1,
        break_anywhere: true,
        ..Default::default()
    };
    let galley = ui.painter().layout_job(job);
    let text_pos = pos2(text_left, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, text_color);

    let on_arrow = response.interact_pointer_pos().is_some_and(|pos| arrow_rect.contains(pos));
    if response.clicked() {
        if on_arrow && row.has_children {
            actions.push(Action::SetExpanded(row.id, !row.expanded));
        } else {
            actions.push(Action::Select(Some(row.id)));
        }
    }
    if response.double_clicked() && row.has_children && !on_arrow {
        actions.push(Action::SetExpanded(row.id, !row.expanded));
    }
    response.context_menu(|ui| context_menu(ui, state, row.id, row.has_children, actions));
    if selected && response.has_focus() {
        response.scroll_to_me(Some(Align::Center));
    }
    age.is_some()
}

fn context_menu(ui: &mut egui::Ui, state: &AppState, id: NodeId, has_children: bool, actions: &mut Vec<Action>) {
    let node = state.tree.node(id);
    if ui.button("Copy topic").clicked() {
        ui.ctx().copy_text(state.tree.topic(id));
        ui.close();
    }
    if let Some(entry) = node.last()
        && ui.button("Copy value").clicked()
    {
        ui.ctx().copy_text(String::from_utf8_lossy(&entry.payload).into_owned());
        ui.close();
    }
    if ui.button("Publish to this topic").clicked() {
        actions.push(Action::UseTopicForPublish(id));
        ui.close();
    }
    if has_children {
        ui.separator();
        if ui.button("Expand all below").clicked() {
            actions.push(Action::SetExpandedRecursive(id, true));
            ui.close();
        }
        if ui.button("Collapse all below").clicked() {
            actions.push(Action::SetExpandedRecursive(id, false));
            ui.close();
        }
    }
    ui.separator();
    if node.last().is_some() && ui.button("Delete retained message").clicked() {
        actions.push(Action::DeleteRetained { node: id, recursive: false });
        ui.close();
    }
    if has_children && ui.button("Delete retained messages below…").clicked() {
        actions.push(Action::DeleteRetained { node: id, recursive: true });
        ui.close();
    }
}
