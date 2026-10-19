//! Writing the drawing as an SVG file, for software that takes SVG but not G-code (for example a
//! cloud-driven laser's own app). Units are millimetres, y points down as in the workspace, and each
//! layer becomes a group coloured like the layer: line layers are strokes, fill layers are filled.

use std::fmt::Write as _;
use std::io::Cursor;

use laserfoundry_doc::{Document, LayerMode, ShapeId, ShapeKind, base64};
use laserfoundry_geom::Rect;

use crate::EngineError;

/// Stroke width, in mm, of line layers (a hairline; the cutter's own settings decide the real cut).
const HAIRLINE_MM: f64 = 0.1;

/// The drawing as SVG text. `only` limits it to those shapes; layers whose output is off are left
/// out unless `include_hidden`. Fails when there is nothing to write.
pub fn export(doc: &Document, only: Option<&[ShapeId]>, include_hidden: bool) -> Result<(String, usize), EngineError> {
    let mut bounds: Option<Rect> = None;
    let mut body = String::new();
    let mut count = 0usize;
    for layer in &doc.layers {
        if !layer.output && !include_hidden {
            continue;
        }
        let mut group = String::new();
        for shape in doc.shapes.iter().filter(|s| s.layer == layer.id && only.is_none_or(|ids| ids.contains(&s.id))) {
            let Some(b) = shape.world_bounds().filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite())) else { continue };
            match &shape.kind {
                ShapeKind::Path { path } | ShapeKind::Text { path, .. } => {
                    let mut p = path.clone();
                    p.apply_affine(shape.transform);
                    let _ = writeln!(group, "    <path d=\"{}\"/>", p.to_svg());
                }
                ShapeKind::Image { image, width_mm, height_mm, adjust } => {
                    let gray: Vec<u8> = image.gray.iter().map(|g| adjust.apply(f32::from(*g)).round().clamp(0.0, 255.0) as u8).collect();
                    let Some(img) = image::GrayImage::from_raw(image.width, image.height, gray) else { continue };
                    let mut png = Vec::new();
                    if img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
                        continue;
                    }
                    let c = shape.transform.as_coeffs();
                    let _ = writeln!(
                        group,
                        "    <image width=\"{width_mm}\" height=\"{height_mm}\" transform=\"matrix({} {} {} {} {} {})\" href=\"data:image/png;base64,{}\"/>",
                        c[0],
                        c[1],
                        c[2],
                        c[3],
                        c[4],
                        c[5],
                        base64::encode(&png)
                    );
                }
            }
            bounds = Some(bounds.map_or(b, |a| a.union(b)));
            count += 1;
        }
        if group.is_empty() {
            continue;
        }
        let style = match layer.mode {
            LayerMode::Line => format!("fill=\"none\" stroke=\"{}\" stroke-width=\"{HAIRLINE_MM}\"", attr(&layer.color)),
            LayerMode::Fill => format!("fill=\"{}\" stroke=\"none\" fill-rule=\"evenodd\"", attr(&layer.color)),
            LayerMode::FillAndLine => {
                format!("fill=\"{c}\" stroke=\"{c}\" stroke-width=\"{HAIRLINE_MM}\" fill-rule=\"evenodd\"", c = attr(&layer.color))
            }
        };
        let _ = write!(body, "  <g id=\"layer-{}\" data-name=\"{}\" {style}>\n{group}  </g>\n", layer.id, attr(&layer.name));
    }
    let Some(b) = bounds else { return Err(EngineError::BadParams("there is nothing to export".into())) };
    let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
    let svg = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"{} {} {w} {h}\">\n{body}</svg>\n",
        b.x0, b.y0
    );
    Ok((svg, count))
}

/// Text made safe for an XML attribute.
fn attr(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            other => other.to_string(),
        })
        .collect()
}
