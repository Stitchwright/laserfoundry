//! The engine's side of the AI features: describing the open document for the model, and placing the
//! artwork it draws. The requests themselves are built by `laserfoundry-ai`; the interface sends them.

use std::collections::BTreeMap;

use laserfoundry_doc::{LayerMode, Shape};
use laserfoundry_geom::Affine;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, parse, shapes::finite};
use crate::{EngineError, Session};

/// The most shapes artwork from the model may add (the importer caps its own input).
const MAX_ART_SHAPES: usize = 5_000;
/// The smallest and largest size (longest side, mm) artwork is placed at.
const SIZE_LIMITS: (f64, f64) = (1.0, 5_000.0);

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "ai.context",
        label: "Describe Document for AI",
        menu: "",
        shortcut: None,
        params_doc: "{}  — returns {text}: the machine and the layers in use (mode, speed, power, passes, shape counts) in words, which is what the AI features tell the model about the open document",
        mutates: false,
        run: context,
    },
    CommandSpec {
        id: "ai.prepare_picture",
        label: "Prepare Picture for AI",
        menu: "",
        shortcut: None,
        params_doc: "{path | data_base64}  — a PNG, JPEG, WebP, GIF or BMP shrunk to at most 1568 pixels on its longest side (transparent areas on white) and encoded as a JPEG for the AI; returns {media_type, data_base64, width, height}",
        mutates: false,
        run: prepare_picture,
    },
    CommandSpec {
        id: "ai.place_art",
        label: "Place AI Artwork",
        menu: "",
        shortcut: None,
        params_doc: "{svg, size_mm? (longest side; default: as drawn, but no larger than the bed), x?, y? (top-left in mm; default centred on the bed)}  — adds the SVG's shapes on the layers its colours name (red cut, blue score, black engrave: layer 0 becomes a fill layer if it was empty and all the artwork on it is filled), as one group, selected; returns {ids, layers, width_mm, height_mm}",
        mutates: true,
        run: place_art,
    },
];

/// What the model is told about the document: plain sentences, no instructions.
fn context(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    let m = &s.doc.machine;
    let mut text = format!(
        "Machine: {}, bed {} x {} mm, origin {:?}, G-code dialect {:?}, S maximum {}, dynamic power {}.\n",
        if m.name.is_empty() { "unnamed" } else { &m.name },
        m.bed_width,
        m.bed_height,
        m.origin,
        m.dialect,
        m.max_s,
        if m.dynamic_power { "on" } else { "off" },
    );
    let mut count: BTreeMap<u32, usize> = BTreeMap::new();
    for shape in &s.doc.shapes {
        *count.entry(shape.layer).or_default() += 1;
    }
    if count.is_empty() {
        text.push_str("The document is empty.\n");
    }
    for (id, n) in &count {
        let Some(l) = s.doc.layer(*id) else { continue };
        let mode = match l.mode {
            LayerMode::Line => "outline (line)",
            LayerMode::Fill => "filled (engrave)",
            LayerMode::FillAndLine => "fill and line",
        };
        text.push_str(&format!(
            "Layer {} ({}, colour {}): {mode}, {n} shape{}, speed {} mm/min, power {}-{} %, {} pass{}{}.\n",
            l.id,
            if l.name.is_empty() { "unnamed" } else { &l.name },
            l.color,
            if *n == 1 { "" } else { "s" },
            l.speed,
            l.min_power,
            l.max_power,
            l.passes,
            if l.passes == 1 { "" } else { "es" },
            if l.air_assist { ", air assist on" } else { "" },
        ));
    }
    Ok(json!({ "text": text }))
}

#[derive(Deserialize)]
struct PictureParams {
    path: Option<std::path::PathBuf>,
    data_base64: Option<String>,
}

/// The largest decoded picture accepted, in pixels, and the largest file or base64 text read.
const MAX_PICTURE_PIXELS: u64 = 50_000_000;
const MAX_PICTURE_BYTES: u64 = 64 * 1024 * 1024;

fn prepare_picture(_: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: PictureParams = parse(params)?;
    let bytes = match (&p.path, &p.data_base64) {
        (Some(path), _) => {
            laserfoundry_format::read_capped(path, MAX_PICTURE_BYTES)?.ok_or_else(|| EngineError::Import("the picture is too large".into()))?
        }
        (None, Some(data)) => {
            if data.len() as u64 > MAX_PICTURE_BYTES * 4 / 3 + 16 {
                return Err(EngineError::Import("the picture is too large".into()));
            }
            super::file::decode_data(data)?
        }
        (None, None) => return Err(EngineError::BadParams("give either path or data_base64".into())),
    };
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PICTURE_PIXELS * 8);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| EngineError::Import(e.to_string()))?;
    if u64::from(decoded.width()) * u64::from(decoded.height()) > MAX_PICTURE_PIXELS {
        return Err(EngineError::Import("the picture is too large".into()));
    }
    // Transparent areas become white: the model sees what a person would on paper.
    let mut rgba = decoded.to_rgba8();
    for px in rgba.pixels_mut() {
        let a = u16::from(px.0[3]);
        for c in &mut px.0[..3] {
            *c = ((u16::from(*c) * a + 255 * (255 - a)) / 255) as u8;
        }
        px.0[3] = 255;
    }
    let mut rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
    let side = laserfoundry_ai::IMAGE_SIDE;
    if rgb.width().max(rgb.height()) > side {
        let (w, h) = (rgb.width(), rgb.height());
        let scale = f64::from(side) / f64::from(w.max(h));
        let (nw, nh) = (((f64::from(w) * scale).round() as u32).max(1), ((f64::from(h) * scale).round() as u32).max(1));
        rgb = image::imageops::resize(&rgb, nw, nh, image::imageops::FilterType::Triangle);
    }
    // A high quality first; a lower one if the encoded picture is still too large for the request.
    for quality in [88u8, 70, 50] {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(&rgb).map_err(|e| EngineError::Import(e.to_string()))?;
        let data_base64 = laserfoundry_doc::base64::encode(&out);
        if data_base64.len() <= laserfoundry_ai::MAX_IMAGE_BASE64 {
            return Ok(json!({ "media_type": "image/jpeg", "data_base64": data_base64, "width": rgb.width(), "height": rgb.height() }));
        }
    }
    Err(EngineError::Import("the picture is too large to send".into()))
}

#[derive(Deserialize)]
struct PlaceParams {
    svg: String,
    size_mm: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
}

fn place_art(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: PlaceParams = parse(params)?;
    finite(&[p.size_mm.unwrap_or(1.0), p.x.unwrap_or(0.0), p.y.unwrap_or(0.0)])?;
    if p.size_mm.is_some_and(|v| v < SIZE_LIMITS.0 || v > SIZE_LIMITS.1) {
        return Err(EngineError::BadParams(format!("size_mm must be between {} and {}", SIZE_LIMITS.0, SIZE_LIMITS.1)));
    }
    let svg = laserfoundry_svg::import_svg(p.svg.as_bytes()).map_err(|e| EngineError::Import(e.to_string()))?;
    if svg.paths.is_empty() {
        return Err(EngineError::Import("the artwork has no shapes".into()));
    }
    if svg.paths.len() > MAX_ART_SHAPES {
        return Err(EngineError::Import("the artwork has too many shapes".into()));
    }
    let empty_before: Vec<u32> = (0..laserfoundry_doc::LAYER_COLORS.len() as u32).filter(|l| !s.doc.shapes.iter().any(|sh| sh.layer == *l)).collect();
    let mut ids = Vec::new();
    // Per layer: does all the artwork on it fill its shapes (engraving), or does some of it only outline?
    let mut all_filled: BTreeMap<u32, bool> = BTreeMap::new();
    for ip in svg.paths {
        let layer = ip.color.as_deref().map_or(0, super::file::layer_for_color);
        let entry = all_filled.entry(layer).or_insert(true);
        *entry &= ip.filled;
        ids.push(s.doc.add_shape(Shape::path(ip.name, layer, ip.path)));
    }
    // Size: as drawn unless asked, and never larger than the bed.
    let bounds = s.doc.bounds_of(&ids).ok_or_else(|| EngineError::Import("the artwork has no size".into()))?;
    let longest = bounds.width().max(bounds.height());
    if !longest.is_finite() || longest <= 0.0 {
        return Err(EngineError::Import("the artwork has no size".into()));
    }
    let (bed_w, bed_h) = (s.doc.machine.bed_width, s.doc.machine.bed_height);
    let wanted = p.size_mm.unwrap_or(longest);
    let fit = (bed_w / bounds.width().max(1e-9)).min(bed_h / bounds.height().max(1e-9));
    let scale = (wanted / longest).min(if p.size_mm.is_some() { f64::MAX } else { fit });
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    for id in &ids {
        if let Some(sh) = s.doc.shape_mut(*id) {
            sh.transform = Affine::scale(scale) * sh.transform;
        }
    }
    let bounds = s.doc.bounds_of(&ids).ok_or_else(|| EngineError::Import("the artwork has no size".into()))?;
    let (x, y) = match (p.x, p.y) {
        (Some(x), Some(y)) => (x, y),
        _ => ((bed_w - bounds.width()) / 2.0, (bed_h - bounds.height()) / 2.0),
    };
    for id in &ids {
        if let Some(sh) = s.doc.shape_mut(*id) {
            sh.transform = Affine::translate((x - bounds.x0, y - bounds.y0)) * sh.transform;
        }
    }
    // Engraved art needs a fill layer; only an untouched layer is changed.
    for (layer, filled) in &all_filled {
        if *filled
            && empty_before.contains(layer)
            && let Some(l) = s.doc.layer_mut(*layer)
            && l.mode == LayerMode::Line
        {
            l.mode = LayerMode::Fill;
        }
    }
    if ids.len() > 1 {
        let gid = s.doc.next_id;
        s.doc.next_id = s.doc.next_id.saturating_add(1);
        for id in &ids {
            if let Some(sh) = s.doc.shape_mut(*id) {
                sh.group = Some(gid);
            }
        }
    }
    let placed = s.doc.bounds_of(&ids).ok_or_else(|| EngineError::Import("the artwork has no size".into()))?;
    s.selection = ids.clone();
    Ok(json!({ "ids": ids, "layers": all_filled.keys().collect::<Vec<_>>(), "width_mm": placed.width(), "height_mm": placed.height() }))
}
