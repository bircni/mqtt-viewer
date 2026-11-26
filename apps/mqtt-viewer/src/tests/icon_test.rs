use std::path::Path;

use crate::icon::{MACOS_INSET, rgba, rgba_inset};

fn pixel(pixels: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
    let start = ((y * size + x) * 4) as usize;
    [pixels[start], pixels[start + 1], pixels[start + 2], pixels[start + 3]]
}

#[test]
fn the_icon_is_a_rounded_tile_with_a_topic_tree() {
    let pixels = rgba(256);
    assert_eq!(pixels.len(), 256 * 256 * 4);
    assert_eq!(pixel(&pixels, 256, 0, 0)[3], 0, "corners are transparent");
    assert_eq!(pixel(&pixels, 256, 255, 255)[3], 0);
    assert_eq!(pixel(&pixels, 256, 128, 2)[3], 255, "edges are opaque");
    assert_eq!(pixel(&pixels, 256, 100, 69), [255, 255, 255, 255], "the parent topic is white");
    assert_eq!(pixel(&pixels, 256, 160, 128), [255, 255, 255, 255], "so is the first child");
    assert_eq!(pixel(&pixels, 256, 50, 128)[..3], pixel(&pixels, 256, 220, 128)[..3], "the child is indented");
    assert_eq!(pixel(&pixels, 256, 195, 69), [255, 194, 75, 255], "the live dot is amber");
    let (top, bottom) = (pixel(&pixels, 256, 128, 10), pixel(&pixels, 256, 128, 245));
    assert!(top[2] > bottom[2], "the tile gets darker towards the bottom: {top:?} {bottom:?}");
}

#[test]
fn edges_are_smoothed() {
    let pixels = rgba(64);
    let partly_covered = pixels.as_chunks::<4>().0.iter().filter(|pixel| pixel[3] != 0 && pixel[3] != 255).count();
    assert!(partly_covered > 20, "only {partly_covered} pixels are partly transparent");
}

#[test]
fn the_inset_leaves_a_transparent_margin() {
    let pixels = rgba_inset(256, MACOS_INSET);
    assert_eq!(pixel(&pixels, 256, 128, 12)[3], 0, "above the tile");
    assert_eq!(pixel(&pixels, 256, 12, 128)[3], 0, "left of the tile");
    assert_eq!(pixel(&pixels, 256, 128, 30)[3], 255, "inside the tile");
    assert_eq!(pixel(&pixels, 256, 128, 128), [255, 255, 255, 255], "the tree shrinks with the tile");
}

#[test]
fn any_size_is_supported() {
    assert_eq!(rgba(0).len(), 0);
    assert_eq!(rgba(1).len(), 4);
    assert_eq!(rgba(256).len(), 256 * 256 * 4);
    assert_eq!(rgba_inset(16, 5.0).len(), 16 * 16 * 4, "an absurd inset is limited");
}

fn png(size: u32, inset: f64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgba_inset(size, inset)).unwrap();
    writer.finish().unwrap();
    out
}

/// A Windows icon file. Small sizes are stored as bitmaps, which every tool reads; 256 pixels as PNG.
fn ico() -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = [16, 24, 32, 48, 64, 256]
        .into_iter()
        .map(|size| (size, if size == 256 { png(size, 0.0) } else { bitmap(size) }))
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&u16::try_from(images.len()).unwrap().to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, data) in &images {
        // A width or height of 0 means 256.
        let extent = u8::try_from(*size).unwrap_or(0);
        out.extend_from_slice(&[extent, extent, 0, 0, 1, 0, 32, 0]);
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
        offset += data.len();
    }
    for (_, data) in images {
        out.extend(data);
    }
    out
}

/// One bitmap of an icon file: a header, the pixels as BGRA from the bottom row up, and an (unused, because
/// the pixels carry alpha) one-bit transparency mask.
fn bitmap(size: u32) -> Vec<u8> {
    let pixels = rgba(size);
    let mut out = Vec::new();
    out.extend_from_slice(&40_u32.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    // The height counts the pixels and the mask.
    out.extend_from_slice(&(size * 2).to_le_bytes());
    out.extend_from_slice(&[1, 0, 32, 0]);
    out.extend_from_slice(&[0; 24]);
    for row in pixels.chunks_exact(size as usize * 4).rev() {
        for pixel in row.as_chunks::<4>().0 {
            out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    let mask_row = size.div_ceil(32) as usize * 4;
    out.resize(out.len() + mask_row * size as usize, 0);
    out
}

/// A macOS icon file: a list of PNG images tagged with the slot they fill.
fn icns() -> Vec<u8> {
    let slots: [(&[u8; 4], u32); 10] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"ic11", 32),
        (b"ic12", 64),
        (b"ic07", 128),
        (b"ic13", 256),
        (b"ic08", 256),
        (b"ic14", 512),
        (b"ic09", 512),
        (b"ic10", 1024),
    ];
    let mut body = Vec::new();
    for (tag, size) in slots {
        let data = png(size, MACOS_INSET);
        body.extend_from_slice(tag);
        body.extend_from_slice(&u32::try_from(data.len() + 8).unwrap().to_be_bytes());
        body.extend(data);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&u32::try_from(body.len() + 8).unwrap().to_be_bytes());
    out.extend(body);
    out
}

/// The packages ship icon files, which must show what [`crate::icon`] draws. `scripts/icons.sh` runs this test
/// with `MQTT_VIEWER_UPDATE_ICONS` set to write them.
#[test]
fn the_icon_files_match_the_drawing() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let update = std::env::var_os("MQTT_VIEWER_UPDATE_ICONS").is_some();
    for (name, expected) in [("icon.png", png(512, 0.0)), ("icon.ico", ico()), ("icon.icns", icns())] {
        let path = directory.join(name);
        if update {
            std::fs::write(&path, &expected).unwrap();
        }
        let current = std::fs::read(&path).unwrap_or_default();
        assert!(current == expected, "{} is outdated; run scripts/icons.sh", path.display());
    }
}
