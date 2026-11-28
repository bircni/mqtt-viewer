//! The topic tree. Only the rows inside the viewport are laid out, so the cost per frame does not depend on
//! the number of topics.

use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{
    self, Align, FontId, Rect, RichText, Role, ScrollArea, Sense, Stroke, TextStyle, WidgetInfo, pos2, vec2,
};
use jiff::Timestamp;
use mqtt_viewer_core::connection::ConnectionState;
use mqtt_viewer_core::decode;
use mqtt_viewer_core::tree::{NodeId, Row, TopicTree};

use super::fonts::{self, icon};
use super::theme::{self, Palette};
use crate::format;
use crate::state::{Action, AppState};

const INDENT: f32 = 16.0;
const ARROW_WIDTH: f32 = 18.0;
/// Seconds a row stays tinted after a message.
const FLASH_SECS: f64 = 1.2;
const PREVIEW_CHARS: usize = 120;

pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let total = state.tree.rows().len();
    if state.tree.rows_pending() {
        // Sorting by counts and payload filtering are refreshed on a timer; make sure a frame follows.
        ui.ctx().request_repaint_after(TopicTree::DEFAULT_REFRESH_INTERVAL);
    }
    if !state.tree.filter().is_empty() {
        search_bar(ui, state, actions);
    }
    if total == 0 {
        let (glyph, hint) = if state.tree.stats().topics > 0 {
            (icon::MAGNIFYING_GLASS, "No topic matches the search")
        } else if state.connection.is_active() {
            (icon::BROADCAST, "Waiting for messages…")
        } else {
            (icon::PLUGS, "Connect to a broker to see its topics")
        };
        theme::empty_state(ui, glyph, hint, None);
        return;
    }
    let reveal = std::mem::take(&mut state.reveal_selection)
        .then(|| state.selected.and_then(|id| state.tree.cached_rows().iter().position(|row| row.id == id)))
        .flatten();
    let state = &*state;
    let font = TextStyle::Body.resolve(ui.style());
    let style = RowStyle {
        row_height: ui.fonts_mut(|fonts| fonts.row_height(&font)) + 7.0,
        font,
        palette: Palette::of(ui),
        now: Timestamp::now(),
        path: selected_path(state),
    };
    let mut flashing = false;
    broker_row(ui, state, &style);

    ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, viewport| {
        let row_height = style.row_height;
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
            flashing |= draw_row(ui, state, row, row_rect(index), &style, actions);
        }
    });
    if flashing {
        ui.ctx().request_repaint_after_secs(0.05);
    }
}

/// Values every row of a frame shares.
struct RowStyle {
    font: FontId,
    row_height: f32,
    palette: Palette,
    now: Timestamp,
    /// The selected topic's ancestors; their names are highlighted so the selection's path stands out.
    path: Vec<NodeId>,
}

fn selected_path(state: &AppState) -> Vec<NodeId> {
    let mut path = Vec::new();
    let mut current = state.selected.filter(|id| state.tree.contains(*id)).and_then(|id| state.tree.node(id).parent());
    while let Some(id) = current.filter(|id| *id != TopicTree::ROOT) {
        path.push(id);
        current = state.tree.node(id).parent();
    }
    path
}

/// The broker as the tree's root: where the topics come from and how many there are.
fn broker_row(ui: &mut egui::Ui, state: &AppState, style: &RowStyle) {
    let palette = &style.palette;
    let stats = state.tree.stats();
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(theme::icon(icon::BROADCAST, palette.accent));
        let host = state.profile.as_ref().map_or("Topics", |profile| profile.host.as_str());
        ui.label(theme::strong(host, 13.5).color(palette.text_strong));
        ui.label(
            RichText::new(format!(
                "({}, {})",
                format::plural(stats.topics as u64, "topic"),
                format::plural(stats.messages, "message")
            ))
            .color(palette.text_weak),
        );
    });
}

/// Above the results of a search: what kind of search it is and the bulk action on its matches.
fn search_bar(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    let palette = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(theme::icon(icon::MAGNIFYING_GLASS, palette.text_weak));
        let kind = if state.tree.filter().is_wildcard() { "Topic filter" } else { "Search results" };
        ui.label(RichText::new(kind).color(palette.text_weak));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let enabled = state.connection == ConnectionState::Connected;
            let delete = ui
                .add_enabled_ui(enabled, |ui| theme::icon_button(ui, icon::TRASH, "Delete matching…"))
                .inner
                .on_hover_text("Clear the retained messages of every matching topic")
                .on_disabled_hover_text("Connect to a broker to delete retained messages");
            if delete.clicked() {
                actions.push(Action::DeleteMatches);
            }
        });
    });
    ui.add_space(4.0);
}

/// Draws one row and returns whether it is still fading out its activity highlight.
fn draw_row(
    ui: &egui::Ui,
    state: &AppState,
    row: Row,
    rect: Rect,
    style: &RowStyle,
    actions: &mut Vec<Action>,
) -> bool {
    let palette = &style.palette;
    let node = state.tree.node(row.id);
    let response = ui.interact(rect, ui.scope_id().with(row.id), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, node.name()));
    let selected = state.selected == Some(row.id);

    let age = state
        .activity_age(row.id, style.now)
        .filter(|age| state.config.settings.highlight_activity && (0.0..FLASH_SECS).contains(age));
    if selected {
        ui.painter().rect_filled(rect, 5.0, palette.selection);
        let marker = Rect::from_min_size(pos2(rect.left(), rect.top() + 4.0), vec2(3.0, rect.height() - 8.0));
        ui.painter().rect_filled(marker, 1.5, palette.accent);
    } else if let Some(age) = age {
        #[expect(clippy::cast_possible_truncation, reason = "a fraction between 0 and 1")]
        let strength = (1.0 - age / FLASH_SECS).clamp(0.0, 1.0) as f32;
        ui.painter().rect_filled(rect, 5.0, palette.activity.gamma_multiply(strength));
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, palette.control);
    }

    #[expect(clippy::cast_precision_loss, reason = "topic depth is small")]
    let left = rect.left() + 8.0 + INDENT * row.depth as f32;
    let arrow_rect = Rect::from_min_size(pos2(left, rect.top()), vec2(ARROW_WIDTH, rect.height()));
    if row.has_children {
        let center = arrow_rect.center();
        let half = style.font.size * 0.26;
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
        ui.painter().add(egui::Shape::convex_polygon(points, palette.text_weak, Stroke::NONE));
    }

    let on_path = style.path.contains(&row.id);
    draw_label(
        ui,
        state,
        row,
        Rect::from_x_y_ranges(arrow_rect.right()..=rect.right(), rect.y_range()),
        style,
        on_path,
    );

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

/// Draws `name = value (n topics, m messages)` into `rect`, the name on a chip when it is on the selection's
/// path.
fn draw_label(ui: &egui::Ui, state: &AppState, row: Row, rect: Rect, style: &RowStyle, on_path: bool) {
    let palette = &style.palette;
    let font = &style.font;
    let format = |font_id: FontId, color| TextFormat { font_id, color, valign: Align::Center, ..Default::default() };
    let node = state.tree.node(row.id);
    let painter = ui.painter();

    let name = if node.name().is_empty() { "(empty)" } else { node.name() };
    let name_font = if row.has_children || on_path { fonts::semibold(font.size) } else { fonts::medium(font.size) };
    let name_galley = painter.layout_no_wrap(name.to_owned(), name_font, palette.text_strong);
    let name_pos = pos2(rect.left(), rect.center().y - name_galley.size().y / 2.0);
    if on_path {
        let chip = Rect::from_min_size(name_pos, name_galley.size()).expand2(vec2(5.0, 1.5));
        painter.rect_filled(chip, 5.0, palette.selection);
    }
    let after_name = name_pos.x + name_galley.size().x;
    painter.galley(name_pos, name_galley, palette.text_strong);

    let mut job = LayoutJob::default();
    if let Some(entry) = node.last() {
        job.append("=", 6.0, format(font.clone(), palette.text_weak));
        let preview = decode::preview(&entry.payload, PREVIEW_CHARS);
        job.append(&preview, 6.0, format(font.clone(), palette.text));
    }
    if row.has_children {
        let summary = format!(
            "({}, {})",
            format::plural(node.subtree_topics() as u64, "topic"),
            format::plural(node.subtree_messages(), "message")
        );
        job.append(&summary, 6.0, format(FontId::proportional(font.size - 1.0), palette.text_weak));
    }
    job.wrap = TextWrapping {
        max_width: (rect.right() - after_name - 8.0).max(0.0),
        max_rows: 1,
        break_anywhere: true,
        ..Default::default()
    };
    let galley = painter.layout_job(job);
    painter.galley(pos2(after_name, rect.center().y - galley.size().y / 2.0), galley, palette.text);
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
