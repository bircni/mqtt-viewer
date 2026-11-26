//! The application icon, drawn in code so that every size comes from one definition.
//!
//! The window icon is rendered from here at startup. The files in `assets/` (PNG, ICO and ICNS, for the
//! packages) are generated from the same code by `scripts/icons.sh`, and a test fails when they fall behind.
//!
//! Shapes are described by signed distance functions in a unit square, which gives clean edges at any size.
//! Only `+ - * /` and `sqrt` are used: those are exactly rounded everywhere, so every platform renders the
//! same bytes.

/// Part of the canvas left empty on each side of the macOS icon. Apple's icon grid puts an 824 pixel tile on a
/// 1024 pixel canvas; the Dock and Finder expect that margin.
pub const MACOS_INSET: f64 = 100.0 / 1024.0;

type Point = (f64, f64);
type Rgb = [f64; 3];

/// Tile colour at the top and at the bottom.
const TILE: [Rgb; 2] = [[43.0, 163.0, 184.0], [20.0, 90.0, 107.0]];
/// Colour of the topic tree.
const GLYPH: Rgb = [255.0, 255.0, 255.0];
/// Colour of the dot that stands for a live value.
const ACCENT: Rgb = [255.0, 194.0, 75.0];

/// Corner radius of the tile.
const TILE_RADIUS: f64 = 0.225;
/// The topics: pills as (centre, half extent). The first is the parent, the others its children.
const TOPICS: [(Point, Point); 3] =
    [((0.42, 0.27), (0.20, 0.058)), ((0.62, 0.50), (0.18, 0.058)), ((0.57, 0.73), (0.13, 0.058))];
/// The lines from the parent to its children, as (start, end).
const BRANCHES: [(Point, Point); 3] =
    [((0.30, 0.33), (0.30, 0.73)), ((0.30, 0.50), (0.44, 0.50)), ((0.30, 0.73), (0.44, 0.73))];
/// Half the width of a branch line.
const BRANCH_HALF_WIDTH: f64 = 0.02;
/// How strongly the branches cover the tile; a little less than the topics, so the topics stand out.
const BRANCH_OPACITY: f64 = 0.7;
/// The live dot, as (centre, radius).
const DOT: (Point, f64) = ((0.76, 0.27), 0.058);

/// The icon as `size` × `size` RGBA pixels, filling the whole canvas.
#[must_use]
pub fn rgba(size: u32) -> Vec<u8> {
    rgba_inset(size, 0.0)
}

/// The icon as `size` × `size` RGBA pixels with `inset` (a fraction of the size, below one half) left
/// transparent on every side.
#[must_use]
pub fn rgba_inset(size: u32, inset: f64) -> Vec<u8> {
    let extent = f64::from(size.max(1));
    let tile = 1.0 - 2.0 * inset.clamp(0.0, 0.49);
    // Pixels per unit of the tile's coordinate system.
    let scale = extent * tile;
    let mut pixels = Vec::with_capacity((size as usize).pow(2) * 4);
    for row in 0..size {
        for column in 0..size {
            let at =
                (((f64::from(column) + 0.5) / extent - inset) / tile, ((f64::from(row) + 0.5) / extent - inset) / tile);
            let coverage = |distance: f64| (0.5 - distance * scale).clamp(0.0, 1.0);

            let mut colour = mix(TILE[0], TILE[1], at.1.clamp(0.0, 1.0));
            let branches = BRANCHES
                .iter()
                .map(|(start, end)| coverage(segment(at, *start, *end) - BRANCH_HALF_WIDTH))
                .fold(0.0, f64::max);
            colour = mix(colour, GLYPH, branches * BRANCH_OPACITY);
            let topics = TOPICS
                .iter()
                .map(|(centre, half)| coverage(rounded_box(at, *centre, *half, half.1)))
                .fold(0.0, f64::max);
            colour = mix(colour, GLYPH, topics);
            colour = mix(colour, ACCENT, coverage(length(at.0 - DOT.0.0, at.1 - DOT.0.1) - DOT.1));

            let alpha = coverage(rounded_box(at, (0.5, 0.5), (0.5, 0.5), TILE_RADIUS));
            pixels.extend_from_slice(&[byte(colour[0]), byte(colour[1]), byte(colour[2]), byte(alpha * 255.0)]);
        }
    }
    pixels
}

/// Blends `over` onto `under` with the given opacity.
fn mix(under: Rgb, over: Rgb, opacity: f64) -> Rgb {
    [0, 1, 2].map(|channel| under[channel] + (over[channel] - under[channel]) * opacity)
}

#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the range of a byte")]
fn byte(value: f64) -> u8 {
    (value + 0.5).clamp(0.0, 255.0) as u8
}

#[expect(clippy::imprecise_flops, reason = "`hypot` is not rounded the same on every platform, `sqrt` is")]
fn length(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

/// Signed distance from `at` to a box with rounded corners; negative inside.
fn rounded_box(at: Point, centre: Point, half: Point, radius: f64) -> f64 {
    let x = (at.0 - centre.0).abs() - (half.0 - radius);
    let y = (at.1 - centre.1).abs() - (half.1 - radius);
    length(x.max(0.0), y.max(0.0)) + x.max(y).min(0.0) - radius
}

/// Distance from `at` to the line segment between `start` and `end`.
fn segment(at: Point, start: Point, end: Point) -> f64 {
    let dot = |a: Point, b: Point| a.0 * b.0 + a.1 * b.1;
    let along = (end.0 - start.0, end.1 - start.1);
    let offset = (at.0 - start.0, at.1 - start.1);
    let position = (dot(offset, along) / dot(along, along)).clamp(0.0, 1.0);
    length(offset.0 - along.0 * position, offset.1 - along.1 * position)
}
