//! SVG import: shapes, groups and transforms are resolved by `usvg`, then converted to mm paths.
//! Text is turned into outlines with the fonts installed on the machine (when there are any), and
//! embedded raster pictures are returned as encoded bytes for the caller to decode.

use laserfoundry_geom::{Affine, BezPath};
use thiserror::Error;
use usvg::tiny_skia_path::PathSegment;

/// CSS pixels (96 per inch) to millimetres.
const MM_PER_PX: f64 = 25.4 / 96.0;
/// Largest SVG we will parse.
pub const MAX_SVG_BYTES: usize = 50 * 1024 * 1024;
/// Most bytes of text (all `<text>` elements together) we will lay out. Text layout takes time that grows faster than the
/// length (about 0.6 s at 20,000 characters, 7 s at 80,000 and minutes past a million), so a file full of text is refused.
pub const MAX_TEXT_BYTES: usize = 40_000;
/// Cap on imported paths.
pub const MAX_PATHS: usize = 100_000;

#[derive(Debug, Error)]
pub enum SvgError {
    #[error("SVG is larger than {} MB", MAX_SVG_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("SVG has more than {MAX_TEXT_BYTES} bytes of text; convert the text to paths first")]
    TooMuchText,
    #[error("could not parse SVG: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedPath {
    pub name: String,
    /// In mm, y down, relative to the SVG's top-left corner.
    pub path: BezPath,
    /// `#rrggbb` of the stroke (preferred) or fill, when the SVG specifies one.
    pub color: Option<String>,
    pub filled: bool,
}

/// An embedded raster picture (PNG, JPEG, GIF or WebP data).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedImage {
    pub name: String,
    pub bytes: Vec<u8>,
    /// The picture's size in mm and where it sits: maps its own mm space (0..width, 0..height) onto
    /// the SVG's mm space (y down, from the top-left corner).
    pub width_mm: f64,
    pub height_mm: f64,
    pub transform: Affine,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SvgImport {
    pub width_mm: f64,
    pub height_mm: f64,
    pub paths: Vec<ImportedPath>,
    pub images: Vec<ImportedImage>,
}

/// Cap on imported pictures and on the bytes they may add up to.
pub const MAX_IMAGES: usize = 64;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

pub fn import_svg(data: &[u8]) -> Result<SvgImport, SvgError> {
    if data.len() > MAX_SVG_BYTES {
        return Err(SvgError::TooLarge);
    }
    if text_bytes(data) > MAX_TEXT_BYTES {
        return Err(SvgError::TooMuchText);
    }
    let mut options = usvg::Options::default();
    // Installed fonts are only looked up when the file has text to draw (loading them takes a while).
    if data.windows(5).any(|w| w == b"<text") {
        load_system_fonts(&mut options);
    }
    let tree = usvg::Tree::from_data(data, &options).map_err(|e| SvgError::Parse(e.to_string()))?;
    let size = tree.size();
    let mut found = Found::default();
    collect(tree.root(), &mut found);
    Ok(SvgImport {
        width_mm: f64::from(size.width()) * MM_PER_PX,
        height_mm: f64::from(size.height()) * MM_PER_PX,
        paths: found.paths,
        images: found.images,
    })
}

/// Bytes between each `<text` and its `</text>` (to the end of the data when there is none).
fn text_bytes(data: &[u8]) -> usize {
    let find = |from: usize, needle: &[u8]| data.get(from..).and_then(|rest| rest.windows(needle.len()).position(|w| w == needle)).map(|i| from + i);
    let mut total = 0usize;
    let mut at = 0usize;
    while let Some(start) = find(at, b"<text") {
        let end = find(start, b"</text>").map_or(data.len(), |e| e + b"</text>".len());
        total = total.saturating_add(end - start);
        at = end;
    }
    total
}

#[cfg(not(target_arch = "wasm32"))]
fn load_system_fonts(options: &mut usvg::Options<'_>) {
    std::sync::Arc::make_mut(&mut options.fontdb).load_system_fonts();
}

/// A browser has no font files to read.
#[cfg(target_arch = "wasm32")]
fn load_system_fonts(_: &mut usvg::Options<'_>) {}

#[derive(Default)]
struct Found {
    paths: Vec<ImportedPath>,
    images: Vec<ImportedImage>,
    image_bytes: usize,
}

fn collect(group: &usvg::Group, found: &mut Found) {
    for node in group.children() {
        if found.paths.len() >= MAX_PATHS {
            return;
        }
        let out = &mut found.paths;
        match node {
            usvg::Node::Group(g) => collect(g, found),
            // Text arrives already converted to outlines (an empty group without fonts).
            usvg::Node::Text(t) => collect(t.flattened(), found),
            usvg::Node::Image(img) => {
                let bytes = match img.kind() {
                    usvg::ImageKind::PNG(d) | usvg::ImageKind::JPEG(d) | usvg::ImageKind::GIF(d) | usvg::ImageKind::WEBP(d) => d.as_slice(),
                    usvg::ImageKind::SVG(_) => continue,
                };
                if found.images.len() >= MAX_IMAGES || found.image_bytes + bytes.len() > MAX_IMAGE_BYTES {
                    continue;
                }
                let size = img.size();
                let ts = img.abs_transform();
                let m = Affine::new([ts.sx as f64, ts.ky as f64, ts.kx as f64, ts.sy as f64, ts.tx as f64, ts.ty as f64]);
                let transform = Affine::scale(MM_PER_PX) * m * Affine::scale(1.0 / MM_PER_PX);
                found.image_bytes += bytes.len();
                let name = if img.id().is_empty() { format!("image {}", found.images.len() + 1) } else { img.id().to_string() };
                found.images.push(ImportedImage {
                    name,
                    bytes: bytes.to_vec(),
                    width_mm: f64::from(size.width()) * MM_PER_PX,
                    height_mm: f64::from(size.height()) * MM_PER_PX,
                    transform,
                });
            }
            usvg::Node::Path(p) => {
                let ts = p.abs_transform();
                let m = Affine::new([ts.sx as f64, ts.ky as f64, ts.kx as f64, ts.sy as f64, ts.tx as f64, ts.ty as f64]);
                let mut bez = convert(p.data());
                bez.apply_affine(Affine::scale(MM_PER_PX) * m);
                let color = p.stroke().and_then(|s| paint_hex(s.paint())).or_else(|| p.fill().and_then(|f| paint_hex(f.paint())));
                let name = if p.id().is_empty() { format!("path {}", out.len() + 1) } else { p.id().to_string() };
                out.push(ImportedPath { name, path: bez, color, filled: p.fill().is_some() });
            }
        }
    }
}

fn convert(path: &usvg::tiny_skia_path::Path) -> BezPath {
    let mut b = BezPath::new();
    let pt = |p: usvg::tiny_skia_path::Point| (f64::from(p.x), f64::from(p.y));
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => b.move_to(pt(p)),
            PathSegment::LineTo(p) => b.line_to(pt(p)),
            PathSegment::QuadTo(c, p) => b.quad_to(pt(c), pt(p)),
            PathSegment::CubicTo(c1, c2, p) => b.curve_to(pt(c1), pt(c2), pt(p)),
            PathSegment::Close => b.close_path(),
        }
    }
    b
}

fn paint_hex(paint: &usvg::Paint) -> Option<String> {
    match paint {
        usvg::Paint::Color(c) => Some(format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use laserfoundry_geom::{bounds_of_polylines, flatten};

    use super::*;

    const RECT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96">
        <rect x="10" y="20" width="30" height="40" fill="none" stroke="#ff0000"/>
        <g transform="translate(48 0)"><circle cx="20" cy="20" r="10" fill="#0000ff"/></g>
    </svg>"##;

    #[test]
    fn a_file_full_of_text_is_refused_before_it_is_laid_out() {
        // Regression: 2 million characters in one <text> ran for minutes (layout time grows faster than the length).
        let long = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><text font-size=\"10\">{}</text></svg>", "W".repeat(2_000_000));
        let started = std::time::Instant::now();
        assert!(matches!(import_svg(long.as_bytes()), Err(SvgError::TooMuchText)));
        assert!(started.elapsed().as_secs() < 5);
        // Unterminated, and spread over many elements.
        let open = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><text>{}", "W".repeat(100_000));
        assert!(matches!(import_svg(open.as_bytes()), Err(SvgError::TooMuchText)));
        let many = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>", "<text>0123456789</text>".repeat(5_000));
        assert!(matches!(import_svg(many.as_bytes()), Err(SvgError::TooMuchText)));
        // A normal amount of text is fine.
        let short = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text x=\"1\" y=\"10\">Hello</text></svg>";
        assert!(import_svg(short.as_bytes()).is_ok());
    }

    #[test]
    fn imports_shapes_in_mm_with_group_transforms() {
        let svg = import_svg(RECT_SVG.as_bytes()).unwrap();
        assert!((svg.width_mm - 25.4).abs() < 1e-6);
        assert_eq!(svg.paths.len(), 2);
        let rect = bounds_of_polylines(&flatten(&svg.paths[0].path, 0.01)).unwrap();
        assert!((rect.x0 - 10.0 * MM_PER_PX).abs() < 1e-4);
        assert!((rect.height() - 40.0 * MM_PER_PX).abs() < 1e-4);
        assert_eq!(svg.paths[0].color.as_deref(), Some("#ff0000"));
        let circle = bounds_of_polylines(&flatten(&svg.paths[1].path, 0.01)).unwrap();
        assert!((circle.x0 - (48.0 + 10.0) * MM_PER_PX).abs() < 0.01);
        assert!(svg.paths[1].filled);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(import_svg(b"not svg at all").is_err());
        assert!(import_svg(b"").is_err());
        assert!(import_svg(&[0xff, 0xfe, 0x00]).is_err());
    }

    #[test]
    fn huge_input_is_refused_up_front() {
        assert!(matches!(import_svg(&vec![b' '; MAX_SVG_BYTES + 1]), Err(SvgError::TooLarge)));
    }

    #[test]
    fn an_svg_with_no_shapes_imports_as_empty() {
        let svg = import_svg(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"/>"#).unwrap();
        assert!(svg.paths.is_empty() && svg.images.is_empty());
        assert!((svg.width_mm - 10.0 * MM_PER_PX).abs() < 1e-9);
        assert!((svg.height_mm - 20.0 * MM_PER_PX).abs() < 1e-9);
    }

    #[test]
    fn ids_name_paths_and_unnamed_ones_are_numbered() {
        let svg = import_svg(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <rect id="frame" width="5" height="5"/><rect width="2" height="2"/></svg>"#,
        )
        .unwrap();
        assert_eq!(svg.paths.len(), 2);
        assert_eq!(svg.paths[0].name, "frame");
        assert_eq!(svg.paths[1].name, "path 2");
    }

    #[test]
    fn stroke_colour_wins_over_fill_and_non_colour_paints_give_none() {
        let svg = import_svg(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <defs><linearGradient id="g"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient></defs>
                <rect width="4" height="4" fill="#00ff00" stroke="#0000ff"/>
                <rect width="4" height="4" fill="url(#g)"/>
                <rect width="4" height="4" fill="none" stroke="none"/></svg>"##,
        )
        .unwrap();
        assert_eq!(svg.paths[0].color.as_deref(), Some("#0000ff"));
        assert!(svg.paths[0].filled);
        assert_eq!(svg.paths[1].color, None);
        // A shape with neither fill nor stroke draws nothing, so it is dropped or carries no colour; it must not be "filled".
        assert!(svg.paths.get(2).is_none_or(|p| !p.filled && p.color.is_none()));
    }

    #[test]
    fn curves_and_arcs_come_through_as_geometry() {
        let svg = import_svg(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96">
                <path d="M 0 0 C 10 0 20 10 20 20 Q 30 30 40 20 A 10 10 0 0 1 60 20 Z" fill="none" stroke="#000"/></svg>"##,
        )
        .unwrap();
        assert_eq!(svg.paths.len(), 1);
        let b = bounds_of_polylines(&flatten(&svg.paths[0].path, 0.01)).unwrap();
        assert!(b.width() > 50.0 * MM_PER_PX && b.width() < 61.0 * MM_PER_PX);
        assert!(b.x0.is_finite() && b.y1.is_finite());
    }

    #[test]
    fn deeply_nested_groups_do_not_overflow_the_stack() {
        for depth in [100usize, 2_000, 20_000] {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">{}<rect width=\"1\" height=\"1\"/>{}</svg>",
                "<g>".repeat(depth),
                "</g>".repeat(depth)
            );
            // Either result is fine; reaching this line means no crash.
            let _ = import_svg(svg.as_bytes());
        }
    }

    #[test]
    fn hostile_attribute_values_are_errors_or_finite_output() {
        let cases: [&str; 6] = [
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1e999" height="1e999"><rect width="1" height="1"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"><rect width="1" height="1"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="-5" height="-5"><rect width="1" height="1"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="NaN" height="inf"><rect width="1" height="1"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><path d="M 1e308 1e308 L -1e308 1e308 Z"/></svg>"#,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><use href="#a"/><g id="a"><use href="#a"/></g></svg>"##,
        ];
        for case in cases {
            if let Ok(svg) = import_svg(case.as_bytes()) {
                assert!(svg.width_mm.is_finite() && svg.height_mm.is_finite(), "non-finite size for {case}");
            }
        }
    }

    #[test]
    fn a_malformed_embedded_picture_never_panics_and_valid_ones_are_sane() {
        // A deliberately incomplete PNG (no valid IDAT/CRC), written out byte by byte.
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60,
            0x60, 0x60, 0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0x5e, 0xf3, 0x2a, 0x3a, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
        ];
        let b64 = base64_encode(PNG);
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"96\" height=\"96\">\
             <image id=\"pic\" x=\"10\" y=\"10\" width=\"48\" height=\"48\" xlink:href=\"data:image/png;base64,{b64}\"/></svg>"
        );
        // The truncated PNG may be rejected by the decoder; the contract under test is "no panic, and sane output when accepted".
        if let Ok(import) = import_svg(svg.as_bytes()) {
            for img in &import.images {
                assert!(img.width_mm.is_finite() && img.height_mm.is_finite());
                assert!(!img.bytes.is_empty());
            }
        }
    }

    fn base64_encode(data: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    let idx = ((n >> (18 - 6 * i)) & 63) as usize;
                    out.push(ALPHABET.get(idx).copied().map_or('A', char::from));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
}
