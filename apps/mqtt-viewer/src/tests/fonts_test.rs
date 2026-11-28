//! The bundled fonts: every icon the UI names must be in the cut-down icon font.

use eframe::egui::{FontFamily, FontId, vec2};
use egui_kittest::Harness;

use super::state;
use crate::state::AppState;
use crate::ui::fonts::{self, icon};

#[test]
fn every_icon_is_in_the_icon_font() {
    let harness = Harness::builder()
        .with_size(vec2(400.0, 300.0))
        .build_ui_state(|ui, state: &mut AppState| crate::ui::root(ui, state), state());
    let font = FontId::new(14.0, FontFamily::Name(fonts::ICONS.into()));
    let missing: Vec<_> = icon::ALL
        .iter()
        .filter(|glyph| !harness.ctx.fonts_mut(|fonts| fonts.has_glyphs(&font, glyph)))
        .map(|glyph| format!("U+{:04X}", glyph.chars().next().map_or(0, u32::from)))
        .collect();
    assert!(missing.is_empty(), "cut the icon font again (fonts/README.md); missing: {missing:?}");
}

#[test]
fn text_uses_inter_with_the_extra_weights() {
    let definitions = fonts::definitions();
    let first = |family: FontFamily| definitions.families.get(&family).and_then(|fonts| fonts.first()).cloned();
    assert_eq!(first(FontFamily::Proportional).as_deref(), Some("inter"));
    assert_eq!(first(FontFamily::Name(fonts::SEMIBOLD.into())).as_deref(), Some("inter-semibold"));
    assert_eq!(first(FontFamily::Name(fonts::MEDIUM.into())).as_deref(), Some("inter-medium"));
}
