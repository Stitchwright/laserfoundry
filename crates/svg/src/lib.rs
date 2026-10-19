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
/// Cap on imported paths.
pub const MAX_PATHS: usize = 100_000;

#[derive(Debug, Error)]
pub enum SvgError {
    #[error("SVG is larger than {} MB", MAX_SVG_BYTES / 1024 / 1024)]
    TooLarge,
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
}
