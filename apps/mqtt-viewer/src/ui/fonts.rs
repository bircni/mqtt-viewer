//! The bundled fonts: Inter for text and a cut-down Phosphor for icons. See `fonts/README.md`.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId};

/// Inter in its semibold weight, for headings and emphasis.
pub const SEMIBOLD: &str = "semibold";
/// Inter in its medium weight, for buttons and labels that need a little more presence.
pub const MEDIUM: &str = "medium";
/// The icon font alone, without any fallback.
pub const ICONS: &str = "icons";

/// Phosphor icons, as characters for [`egui::RichText`].
pub mod icon {
    pub const BROADCAST: &str = "\u{E0F2}";
    pub const BROOM: &str = "\u{EC54}";
    pub const CARET_DOWN: &str = "\u{E136}";
    pub const CARET_RIGHT: &str = "\u{E13A}";
    pub const CHART_LINE: &str = "\u{E154}";
    pub const CLOCK_COUNTER_CLOCKWISE: &str = "\u{E1A0}";
    pub const COPY: &str = "\u{E1CA}";
    pub const DATABASE: &str = "\u{E1DE}";
    pub const EXPORT: &str = "\u{EAF0}";
    pub const FLOPPY_DISK: &str = "\u{E248}";
    pub const FOLDER_OPEN: &str = "\u{E256}";
    pub const GEAR_SIX: &str = "\u{E272}";
    pub const HASH: &str = "\u{E2A2}";
    pub const INFO: &str = "\u{E2CE}";
    pub const LOCK_SIMPLE: &str = "\u{E308}";
    pub const MAGNIFYING_GLASS: &str = "\u{E30C}";
    pub const PAPER_PLANE_TILT: &str = "\u{E398}";
    pub const PAUSE: &str = "\u{E39E}";
    pub const PLAY: &str = "\u{E3D0}";
    pub const PLUGS: &str = "\u{EB56}";
    pub const PLUGS_CONNECTED: &str = "\u{EB5A}";
    pub const PLUS: &str = "\u{E3D4}";
    pub const RECORD: &str = "\u{E3EE}";
    pub const SHIELD_CHECK: &str = "\u{E40C}";
    pub const STOP: &str = "\u{E46C}";
    pub const TRASH: &str = "\u{E4A6}";
    pub const TREE_STRUCTURE: &str = "\u{E67C}";
    pub const WARNING: &str = "\u{E4E0}";
    pub const X: &str = "\u{E4F6}";
    pub const CLOCK: &str = "\u{E19A}";
    pub const PLAY_CIRCLE: &str = "\u{E3D2}";

    /// Every icon above, for the test that checks the font contains them.
    #[cfg(test)]
    pub const ALL: [&str; 31] = [
        BROADCAST,
        BROOM,
        CARET_DOWN,
        CARET_RIGHT,
        CHART_LINE,
        CLOCK_COUNTER_CLOCKWISE,
        COPY,
        DATABASE,
        EXPORT,
        FLOPPY_DISK,
        FOLDER_OPEN,
        GEAR_SIX,
        HASH,
        INFO,
        LOCK_SIMPLE,
        MAGNIFYING_GLASS,
        PAPER_PLANE_TILT,
        PAUSE,
        PLAY,
        PLUGS,
        PLUGS_CONNECTED,
        PLUS,
        RECORD,
        SHIELD_CHECK,
        STOP,
        TRASH,
        TREE_STRUCTURE,
        WARNING,
        X,
        CLOCK,
        PLAY_CIRCLE,
    ];
}

/// egui's fonts with Inter in front for text, Phosphor behind it for icons, and the two extra weights.
#[must_use]
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8], tweak: egui::FontTweak| {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes).tweak(tweak)));
    };
    add("inter", include_bytes!("../../fonts/Inter-Regular.ttf"), egui::FontTweak::default());
    add("inter-medium", include_bytes!("../../fonts/Inter-Medium.ttf"), egui::FontTweak::default());
    add("inter-semibold", include_bytes!("../../fonts/Inter-SemiBold.ttf"), egui::FontTweak::default());
    // Icons sit slightly low next to Inter's capitals without the offset.
    add(
        "phosphor",
        include_bytes!("../../fonts/Phosphor.ttf"),
        egui::FontTweak { scale: 1.1, y_offset_factor: 0.08, ..Default::default() },
    );

    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with = |first: &str| {
        let mut family = vec![first.to_owned(), "phosphor".to_owned()];
        family.extend(fallbacks.iter().cloned());
        family
    };
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), with("inter-semibold"));
    fonts.families.insert(FontFamily::Name(MEDIUM.into()), with("inter-medium"));
    fonts.families.insert(FontFamily::Name(ICONS.into()), vec!["phosphor".to_owned()]);
    fonts.families.insert(FontFamily::Proportional, with("inter"));
    fonts.families.entry(FontFamily::Monospace).or_default().push("phosphor".to_owned());
    fonts
}

/// Inter semibold at this size.
#[must_use]
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Inter medium at this size.
#[must_use]
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}
