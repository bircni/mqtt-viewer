//! The window icon, drawn in code so the repository needs no binary assets.

/// Background colour (RGB).
const BACKGROUND: [u8; 3] = [0x1f, 0x7a, 0x8c];
/// Foreground colour (RGB).
const FOREGROUND: [u8; 3] = [0xff, 0xff, 0xff];

/// The icon as `size` × `size` RGBA pixels: a rounded square with three indented bars, a small topic tree.
#[must_use]
pub fn rgba(size: u32) -> Vec<u8> {
    let extent = f64::from(size.max(1));
    let radius = extent * 0.22;
    // Bars as (left, top, right, bottom) in fractions of the icon size.
    let bars = [(0.20, 0.24, 0.80, 0.36), (0.34, 0.44, 0.80, 0.56), (0.34, 0.64, 0.66, 0.76)];
    let mut pixels = Vec::with_capacity((size as usize).pow(2) * 4);
    for row in 0..size {
        for column in 0..size {
            let (x, y) = (f64::from(column) + 0.5, f64::from(row) + 0.5);
            // Distance from the pixel to the square shrunk by the corner radius gives the rounded outline.
            let dx = (radius - x).max(x - (extent - radius)).max(0.0);
            let dy = (radius - y).max(y - (extent - radius)).max(0.0);
            let inside = dx.hypot(dy) <= radius;
            let (fx, fy) = (x / extent, y / extent);
            let on_bar =
                bars.iter().any(|(left, top, right, bottom)| fx >= *left && fx < *right && fy >= *top && fy < *bottom);
            let colour = if on_bar { FOREGROUND } else { BACKGROUND };
            pixels.extend_from_slice(&[colour[0], colour[1], colour[2], if inside { 0xff } else { 0x00 }]);
        }
    }
    pixels
}
