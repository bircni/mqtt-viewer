//! Colours, the egui style, theme handling and the small building blocks every view shares.

use eframe::egui::{
    self, Button, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Response, RichText, Sense, Shadow, Stroke,
    TextStyle, ThemePreference, Vec2, vec2,
};
use mqtt_viewer_core::config::Theme;

use super::fonts;

/// Corner radius of buttons, inputs and rows.
const RADIUS: u8 = 6;
/// Corner radius of cards and menus.
const RADIUS_LARGE: u8 = 10;
/// Corner radius of dialogs.
const RADIUS_DIALOG: u8 = 14;

/// The colours of one theme. Widgets, cards and panels are all built from these.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Behind the topic tree.
    pub base: Color32,
    /// Toolbar, sidebar, status bar, charts and dialog side rails.
    pub panel: Color32,
    /// The app bar at the top, tinted with the accent.
    pub bar: Color32,
    /// Cards inside the panels, dialogs and menus.
    pub surface: Color32,
    /// Text inputs, code, plots and the track of segmented controls.
    pub sunken: Color32,
    /// The selected segment of a segmented control.
    pub raised: Color32,
    /// Idle buttons.
    pub control: Color32,
    /// Hovered buttons.
    pub control_hover: Color32,
    /// Pressed buttons.
    pub control_active: Color32,
    /// Separators and outlines.
    pub border: Color32,
    /// Body text.
    pub text: Color32,
    /// Headings and selected items.
    pub text_strong: Color32,
    /// Secondary text: hints, units, counts.
    pub text_weak: Color32,
    /// Primary actions, focus and the selected row's marker.
    pub accent: Color32,
    /// Background of selected rows and text selection.
    pub selection: Color32,
    /// Text on top of [`Self::accent`] and [`Self::error`].
    pub on_accent: Color32,
    /// Flash colour for rows that just received a message.
    pub activity: Color32,
    /// Added lines in a diff.
    pub added: Color32,
    /// Removed lines in a diff.
    pub removed: Color32,
    /// JSON keys.
    pub key: Color32,
    /// JSON strings and payload previews.
    pub string: Color32,
    /// JSON numbers.
    pub number: Color32,
    /// JSON literals.
    pub literal: Color32,
    /// Connected indicator.
    pub ok: Color32,
    /// Reconnecting indicator and warnings.
    pub warn: Color32,
    /// Errors and the disconnected indicator.
    pub error: Color32,
    shadow: Color32,
}

const DARK: Palette = Palette {
    base: Color32::from_rgb(14, 15, 18),
    panel: Color32::from_rgb(20, 21, 25),
    bar: Color32::from_rgb(25, 23, 42),
    surface: Color32::from_rgb(26, 27, 32),
    sunken: Color32::from_rgb(12, 13, 16),
    raised: Color32::from_rgb(46, 48, 56),
    control: Color32::from_rgb(34, 36, 42),
    control_hover: Color32::from_rgb(43, 45, 53),
    control_active: Color32::from_rgb(53, 56, 66),
    border: Color32::from_rgb(38, 40, 47),
    text: Color32::from_rgb(200, 203, 211),
    text_strong: Color32::from_rgb(244, 245, 248),
    text_weak: Color32::from_rgb(122, 126, 138),
    accent: Color32::from_rgb(124, 118, 255),
    selection: Color32::from_rgb(40, 38, 82),
    on_accent: Color32::WHITE,
    activity: Color32::from_rgb(28, 70, 56),
    added: Color32::from_rgb(110, 214, 160),
    removed: Color32::from_rgb(248, 122, 130),
    key: Color32::from_rgb(150, 170, 255),
    string: Color32::from_rgb(140, 214, 170),
    number: Color32::from_rgb(246, 190, 120),
    literal: Color32::from_rgb(214, 156, 250),
    ok: Color32::from_rgb(64, 204, 140),
    warn: Color32::from_rgb(246, 188, 72),
    error: Color32::from_rgb(244, 98, 108),
    shadow: Color32::from_black_alpha(140),
};

const LIGHT: Palette = Palette {
    base: Color32::from_rgb(252, 252, 253),
    panel: Color32::from_rgb(245, 246, 248),
    bar: Color32::from_rgb(240, 238, 253),
    surface: Color32::from_rgb(255, 255, 255),
    sunken: Color32::from_rgb(240, 241, 244),
    raised: Color32::from_rgb(255, 255, 255),
    control: Color32::from_rgb(236, 237, 241),
    control_hover: Color32::from_rgb(226, 228, 233),
    control_active: Color32::from_rgb(214, 216, 223),
    border: Color32::from_rgb(226, 228, 233),
    text: Color32::from_rgb(52, 55, 64),
    text_strong: Color32::from_rgb(15, 17, 22),
    text_weak: Color32::from_rgb(120, 124, 135),
    accent: Color32::from_rgb(92, 82, 232),
    selection: Color32::from_rgb(232, 230, 255),
    on_accent: Color32::WHITE,
    activity: Color32::from_rgb(208, 242, 224),
    added: Color32::from_rgb(20, 130, 75),
    removed: Color32::from_rgb(200, 40, 55),
    key: Color32::from_rgb(70, 70, 210),
    string: Color32::from_rgb(25, 125, 80),
    number: Color32::from_rgb(185, 95, 0),
    literal: Color32::from_rgb(140, 50, 195),
    ok: Color32::from_rgb(20, 150, 90),
    warn: Color32::from_rgb(190, 120, 0),
    error: Color32::from_rgb(210, 45, 60),
    shadow: Color32::from_black_alpha(36),
};

impl Palette {
    /// The palette matching the UI's current visuals.
    pub fn of(ui: &egui::Ui) -> Self {
        if ui.visuals().dark_mode { DARK } else { LIGHT }
    }
}

/// Applies the configured theme, installing the app's fonts and styles on first use.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let installed = egui::Id::unique("mqtt-viewer-style");
    if !ctx.data(|data| data.get_temp::<bool>(installed).unwrap_or(false)) {
        ctx.set_fonts(fonts::definitions());
        ctx.style_mut_of(egui::Theme::Dark, |style| configure(style, &DARK));
        ctx.style_mut_of(egui::Theme::Light, |style| configure(style, &LIGHT));
        ctx.data_mut(|data| data.insert_temp(installed, true));
    }
    let preference = match theme {
        Theme::System => ThemePreference::System,
        Theme::Light => ThemePreference::Light,
        Theme::Dark => ThemePreference::Dark,
    };
    if ctx.options(|options| options.theme_preference) != preference {
        ctx.set_theme(preference);
    }
}

fn configure(style: &mut egui::Style, palette: &Palette) {
    style.text_styles = [
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Button, fonts::medium(13.0)),
        (TextStyle::Heading, fonts::semibold(18.0)),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(8.0, 8.0);
    spacing.button_padding = vec2(10.0, 5.0);
    spacing.interact_size.y = 28.0;
    spacing.window_margin = Margin::same(20);
    spacing.menu_margin = Margin::same(6);
    spacing.indent = 18.0;
    spacing.icon_width = 16.0;
    spacing.icon_width_inner = 10.0;
    spacing.icon_spacing = 6.0;
    spacing.combo_height = 320.0;
    spacing.scroll.bar_width = 8.0;
    spacing.scroll.floating = true;

    let visuals = &mut style.visuals;
    visuals.override_text_color = None;
    visuals.weak_text_color = Some(palette.text_weak);
    visuals.panel_fill = palette.panel;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = CornerRadius::same(RADIUS_DIALOG);
    visuals.window_shadow = Shadow { offset: [0, 16], blur: 48, spread: 0, color: palette.shadow };
    visuals.popup_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: palette.shadow };
    visuals.menu_corner_radius = CornerRadius::same(RADIUS_LARGE);
    visuals.window_highlight_topmost = false;
    visuals.faint_bg_color = palette.surface;
    visuals.extreme_bg_color = palette.sunken;
    visuals.text_edit_bg_color = Some(palette.sunken);
    visuals.code_bg_color = palette.sunken;
    visuals.hyperlink_color = palette.accent;
    visuals.warn_fg_color = palette.warn;
    visuals.error_fg_color = palette.error;
    visuals.selection.bg_fill = palette.selection;
    visuals.selection.stroke = Stroke::new(1.0, palette.text_strong);
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);
    visuals.indent_has_left_vline = false;
    visuals.collapsing_header_frame = false;
    visuals.striped = false;
    visuals.slider_trailing_fill = true;

    let radius = CornerRadius::same(RADIUS);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.surface;
    widgets.noninteractive.weak_bg_fill = palette.surface;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.noninteractive.corner_radius = radius;

    let states = [
        (&mut widgets.inactive, palette.control, palette.border, palette.text),
        (&mut widgets.hovered, palette.control_hover, palette.control_active, palette.text_strong),
        (&mut widgets.active, palette.control_active, palette.accent, palette.text_strong),
        (&mut widgets.open, palette.control_hover, palette.control_active, palette.text_strong),
    ];
    for (widget, fill, outline, text) in states {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, outline);
        widget.fg_stroke = Stroke::new(1.5, text);
        widget.corner_radius = radius;
        widget.expansion = 0.0;
    }
}

/// The frame of the toolbar, the sidebar, the status bar and the chart panel.
pub fn panel_frame(ui: &egui::Ui, margin: Margin) -> Frame {
    Frame::new().fill(Palette::of(ui).panel).inner_margin(margin)
}

/// The frame of the app bar at the top.
pub fn bar_frame(ui: &egui::Ui) -> Frame {
    Frame::new().fill(Palette::of(ui).bar).inner_margin(Margin::symmetric(12, 8))
}

/// The frame of the area behind the topic tree.
pub fn base_frame(ui: &egui::Ui) -> Frame {
    Frame::new().fill(Palette::of(ui).base).inner_margin(Margin::symmetric(8, 6))
}

/// A rounded card that groups one section of a panel.
pub fn card(ui: &egui::Ui) -> Frame {
    let palette = Palette::of(ui);
    Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.surface))
        .corner_radius(CornerRadius::same(RADIUS_LARGE))
        .inner_margin(Margin::symmetric(14, 12))
}

/// The frame of a dialog. Dialogs lay out their own margins so side rails can reach the edge.
pub fn dialog_frame(ctx: &egui::Context) -> Frame {
    let style = ctx.global_style();
    Frame::window(&style).inner_margin(Margin::ZERO).corner_radius(CornerRadius::same(RADIUS_DIALOG))
}

/// Text in Inter semibold.
pub fn strong(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(fonts::semibold(size))
}

/// A glyph from the icon font.
pub fn icon(glyph: &str, color: Color32) -> RichText {
    RichText::new(glyph).color(color).size(15.0)
}

/// A button with an icon in front of its text. Its accessible name is the text alone.
pub fn icon_button(ui: &mut egui::Ui, glyph: &str, text: &str) -> Response {
    let color = Palette::of(ui).text_weak;
    ui.add(Button::new((icon(glyph, color), text))).accessible_name(text)
}

/// A frameless button that shows only an icon; `name` is its tooltip and accessible name.
pub fn icon_only(ui: &mut egui::Ui, glyph: &str, name: &str) -> Response {
    let color = Palette::of(ui).text_weak;
    ui.add(Button::new(icon(glyph, color)).frame_when_inactive(false).min_size(vec2(28.0, 28.0)))
        .accessible_name(name)
        .on_hover_text(name)
}

/// A filled button for the main action of a dialog or section.
pub fn primary_button(ui: &egui::Ui, glyph: Option<&str>, text: &str) -> Button<'static> {
    let palette = Palette::of(ui);
    filled(palette.accent, palette.on_accent, glyph, text)
}

/// A filled button for an action that destroys data.
pub fn danger_button(ui: &egui::Ui, glyph: Option<&str>, text: &str) -> Button<'static> {
    let palette = Palette::of(ui);
    filled(palette.error, palette.on_accent, glyph, text)
}

fn filled(fill: Color32, color: Color32, glyph: Option<&str>, text: &str) -> Button<'static> {
    let label = RichText::new(text.to_owned()).color(color);
    let button = match glyph {
        Some(glyph) => Button::new((icon(glyph, color), label)),
        None => Button::new(label),
    };
    button.fill(fill).stroke(Stroke::NONE)
}

/// Adds a filled button and names it by its text alone.
pub fn add_filled(ui: &mut egui::Ui, button: Button<'static>, text: &str) -> Response {
    ui.add(button).accessible_name(text)
}

/// A row of mutually exclusive options on a sunken track. Returns the option clicked this frame.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, current: T, options: &[(T, &str)]) -> Option<T> {
    let palette = Palette::of(ui);
    let mut clicked = None;
    Frame::new().fill(palette.sunken).corner_radius(CornerRadius::same(RADIUS + 1)).inner_margin(Margin::same(2)).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = vec2(10.0, 3.0);
            ui.spacing_mut().interact_size.y = 24.0;
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = palette.raised;
            visuals.selection.stroke = Stroke::new(1.0, palette.text_strong);
            ui.horizontal(|ui| {
                for (option, label) in options {
                    let selected = *option == current;
                    let text =
                        RichText::new(*label).color(if selected { palette.text_strong } else { palette.text_weak });
                    if ui.add(Button::selectable(selected, text)).clicked() {
                        clicked = Some(*option);
                    }
                }
            });
        },
    );
    clicked
}

/// Like [`segmented`], writing the choice straight into `value`.
pub fn segmented_value<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) {
    if let Some(choice) = segmented(ui, *value, options) {
        *value = choice;
    }
}

/// A row of tabs with an underline below the current one. Returns the tab clicked this frame.
pub fn tabs<T: PartialEq + Copy>(ui: &mut egui::Ui, current: T, options: &[(T, &str, &str)]) -> Option<T> {
    let palette = Palette::of(ui);
    let mut clicked = None;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (option, glyph, label) in options {
            let selected = *option == current;
            let color = if selected { palette.text_strong } else { palette.text_weak };
            let glyph_color = if selected { palette.accent } else { palette.text_weak };
            let response = ui
                .add(
                    Button::new((icon(glyph, glyph_color), RichText::new(*label).color(color)))
                        .frame_when_inactive(false),
                )
                .accessible_name(*label);
            if selected {
                let rect = response.rect;
                let line = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 4.0, rect.bottom() + 5.0),
                    egui::pos2(rect.right() - 4.0, rect.bottom() + 7.0),
                );
                ui.painter().rect_filled(line, 1.0, palette.accent);
            }
            if response.clicked() {
                clicked = Some(*option);
            }
        }
    });
    let bottom = row.response.rect.bottom() + 7.0;
    let full = ui.max_rect();
    ui.painter().hline(full.x_range(), bottom, Stroke::new(1.0, palette.border));
    ui.add_space(10.0);
    clicked
}

/// A labelled form field: the label in small text above the widget.
pub fn field<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new(label).font(fonts::medium(12.0)).color(Palette::of(ui).text_weak));
        add(ui)
    })
    .inner
}

/// A small rounded badge for metadata such as QoS or payload size.
pub fn badge(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) -> Response {
    Frame::new()
        .fill(color.gamma_multiply(0.14))
        .corner_radius(CornerRadius::same(RADIUS_LARGE))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| ui.label(RichText::new(text).font(fonts::medium(11.5)).color(color)))
        .response
}

/// A coloured status dot.
pub fn dot(ui: &mut egui::Ui, color: Color32, radius: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(radius * 2.0 + 2.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), radius + 2.5, color.gamma_multiply(0.22));
    ui.painter().circle_filled(rect.center(), radius, color);
    response
}

/// A centred placeholder: a large icon, a title and a hint below it.
pub fn empty_state(ui: &mut egui::Ui, glyph: &str, title: &str, hint: Option<&str>) {
    let palette = Palette::of(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 60.0).max(0.0));
        ui.label(RichText::new(glyph).size(40.0).color(palette.text_weak.gamma_multiply(0.6)));
        ui.add_space(4.0);
        ui.label(strong(title, 15.0).color(palette.text));
        if let Some(hint) = hint {
            ui.label(RichText::new(hint).color(palette.text_weak));
        }
    });
}
