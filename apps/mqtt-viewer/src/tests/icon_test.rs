use crate::icon::rgba;

fn pixel(pixels: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
    let start = (y * size + x) * 4;
    [pixels[start], pixels[start + 1], pixels[start + 2], pixels[start + 3]]
}

#[test]
fn the_icon_is_a_rounded_square_with_bars() {
    let pixels = rgba(64);
    assert_eq!(pixels.len(), 64 * 64 * 4);
    assert_eq!(pixel(&pixels, 64, 0, 0)[3], 0, "corners are transparent");
    assert_eq!(pixel(&pixels, 64, 63, 63)[3], 0);
    assert_eq!(pixel(&pixels, 64, 32, 2)[3], 255, "edges are opaque");
    assert_eq!(pixel(&pixels, 64, 32, 19), [255, 255, 255, 255], "the first bar is white");
    assert_eq!(pixel(&pixels, 64, 32, 26), [0x1f, 0x7a, 0x8c, 255], "between the bars is background");
    assert_eq!(pixel(&pixels, 64, 16, 32)[..3], [0x1f, 0x7a, 0x8c], "the second bar is indented");
}

#[test]
fn any_size_is_supported() {
    assert_eq!(rgba(0).len(), 0);
    assert_eq!(rgba(1).len(), 4);
    assert_eq!(rgba(256).len(), 256 * 256 * 4);
}
