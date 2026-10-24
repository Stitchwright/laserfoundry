//! The LaserFoundry application icon, drawn in code (original artwork, no image files): a laser beam striking a cut line on a
//! dark tile, the same mark as the browser page's favicon. [`render_rgba`] draws it at any size; [`png`], [`ico`] and [`icns`]
//! wrap it in the file formats the desktop packages need.
//!
//! * PNG: https://www.w3.org/TR/png/
//! * ICO with PNG-compressed images: https://learn.microsoft.com/en-us/windows/win32/menurc/icons
//! * ICNS (a type tag, a length and PNG data per size): https://en.wikipedia.org/wiki/Apple_Icon_Image_format

use std::io::Cursor;

use image::{ImageEncoder, codecs::png::PngEncoder};

/// Sizes (pixels) stored in the Windows icon, smallest first. 256 is the largest an ICO can describe.
pub const ICO_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];
/// The Apple icon types and the pixel size of the PNG each holds.
pub const ICNS_SIZES: &[(&[u8; 4], u32)] =
    &[(b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)];
/// The largest size [`render_rgba`] will draw.
pub const MAX_SIZE: u32 = 2048;
/// Space left round the tile in the macOS icon (a fraction of the size), as the platform's icons have.
pub const MAC_INSET: f64 = 0.0625;

const TILE: [u8; 3] = [0x1c, 0x1f, 0x24];
const BEAM: [u8; 3] = [0xff, 0x5a, 0x36];
const SPARK: [u8; 3] = [0xff, 0xd1, 0x66];
const CUT: [u8; 3] = [0x73, 0xc6, 0xff];
/// Subsamples per axis for edge smoothing.
const SUPERSAMPLE: u32 = 4;

/// Draws the icon `size` pixels square (clamped to 1..=[`MAX_SIZE`]) as straight-alpha RGBA, row by row from the top.
/// `inset` is the empty margin round the tile as a fraction of the size (0 fills the square).
pub fn render_rgba(size: u32, inset: f64) -> Vec<u8> {
    let size = size.clamp(1, MAX_SIZE);
    let inset = if inset.is_finite() { inset.clamp(0.0, 0.3) } else { 0.0 };
    let mut out = Vec::with_capacity(size as usize * size as usize * 4);
    let n = SUPERSAMPLE;
    for y in 0..size {
        for x in 0..size {
            let (mut r, mut g, mut b, mut a) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for sy in 0..n {
                for sx in 0..n {
                    let u = (f64::from(x) + (f64::from(sx) + 0.5) / f64::from(n)) / f64::from(size);
                    let v = (f64::from(y) + (f64::from(sy) + 0.5) / f64::from(n)) / f64::from(size);
                    if let Some([cr, cg, cb]) = colour_at((u - inset) / (1.0 - 2.0 * inset), (v - inset) / (1.0 - 2.0 * inset)) {
                        r += f64::from(cr);
                        g += f64::from(cg);
                        b += f64::from(cb);
                        a += 1.0;
                    }
                }
            }
            if a == 0.0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let coverage = a / f64::from(n * n);
                out.extend_from_slice(&[(r / a).round() as u8, (g / a).round() as u8, (b / a).round() as u8, (coverage * 255.0).round() as u8]);
            }
        }
    }
    out
}

/// The icon as a PNG file.
pub fn png(size: u32, inset: f64) -> Vec<u8> {
    let size = size.clamp(1, MAX_SIZE);
    let mut out = Vec::new();
    // Encoding into memory cannot fail for a well-formed buffer; an empty file is the (never expected) fallback.
    if PngEncoder::new(Cursor::new(&mut out)).write_image(&render_rgba(size, inset), size, size, image::ExtendedColorType::Rgba8).is_err() {
        out.clear();
    }
    out
}

/// A Windows `.ico` holding [`ICO_SIZES`], each a PNG (supported since Windows Vista).
pub fn ico() -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = ICO_SIZES.iter().map(|&s| (s, png(s, 0.0))).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, data) in &images {
        // A width or height of 0 means 256.
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

/// A macOS `.icns` holding [`ICNS_SIZES`], with the platform's margin round the tile.
pub fn icns() -> Vec<u8> {
    let mut body = Vec::new();
    for (tag, size) in ICNS_SIZES {
        let data = png(*size, MAC_INSET);
        body.extend_from_slice(*tag);
        body.extend_from_slice(&(data.len() as u32 + 8).to_be_bytes());
        body.extend_from_slice(&data);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// The colour at a point of the unit square, or `None` where it is transparent. Shapes are in the favicon's 32-unit grid.
fn colour_at(u: f64, v: f64) -> Option<[u8; 3]> {
    let (x, y) = (u * 32.0, v * 32.0);
    if !(0.0..=32.0).contains(&x) || !(0.0..=32.0).contains(&y) || !in_rounded_square(x, y, 32.0, 7.0) {
        return None;
    }
    if in_capsule(x, y, (16.0, 3.0), (16.0, 16.0), 1.5) {
        return Some(BEAM);
    }
    if (x - 16.0).hypot(y - 18.0) <= 3.2 {
        return Some(SPARK);
    }
    if in_capsule(x, y, (5.0, 25.0), (27.0, 25.0), 1.5) {
        return Some(CUT);
    }
    Some(TILE)
}

fn in_rounded_square(x: f64, y: f64, side: f64, radius: f64) -> bool {
    let dx = (x - side / 2.0).abs() - (side / 2.0 - radius);
    let dy = (y - side / 2.0).abs() - (side / 2.0 - radius);
    dx.max(0.0).hypot(dy.max(0.0)) <= radius
}

/// A line segment thickened by `radius` with round ends.
fn in_capsule(x: f64, y: f64, a: (f64, f64), b: (f64, f64), radius: f64) -> bool {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 == 0.0 { 0.0 } else { (((x - a.0) * abx + (y - a.1) * aby) / len2).clamp(0.0, 1.0) };
    (x - (a.0 + t * abx)).hypot(y - (a.1 + t * aby)) <= radius
}

#[cfg(test)]
mod tests;
