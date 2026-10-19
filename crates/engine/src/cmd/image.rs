use std::sync::Arc;

use laserfoundry_doc::{LayerId, Shape, ShapeId, ShapeKind};
use laserfoundry_geom::{Affine, Vec2};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, parse};
use crate::{EngineError, Session};

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "image.adjust",
        label: "Adjust Image",
        menu: "Edit",
        shortcut: None,
        params_doc: "{id? (default: the selected image), brightness? (-1..1), contrast? (-1..1), gamma? (0.1..5), negative?: bool, reset?: bool}  — tone is applied before dithering; the picture itself is never changed",
        mutates: true,
        run: adjust,
    },
    CommandSpec {
        id: "image.crop",
        label: "Crop Image",
        menu: "Edit",
        shortcut: None,
        params_doc: "{id?, left?, top?, right?, bottom?}  — fractions (0..0.95) of the picture to cut off each side; the rest stays where it is",
        mutates: true,
        run: crop,
    },
    CommandSpec {
        id: "image.trace",
        label: "Trace Image to Vectors",
        menu: "Edit",
        shortcut: None,
        params_doc: "{id?, threshold? (0..255, default 128: pixels darker than this are traced, after the image's tone adjustments), tolerance? (pixels, default 1), min_area? (pixels squared, default 4), layer?, keep_image? (default true), smooth? (default false: curves instead of straight steps), fit? (0.05..50 pixels: least-squares cubic curves within that distance of the outline, far fewer nodes than smooth; overrides smooth; use tolerance 1 or more so pixel steps are not read as corners), levels? (1..8, default 1: that many nested outlines for light-to-dark tonal bands), colors? (2..8: trace that many palette colours separately, each onto the layer whose colour is nearest; needs a coloured picture, imported with this version; ignores threshold, levels and the tone adjustments; paper-white is left out)}  — adds a path shape that follows the dark areas, on top of the picture",
        mutates: true,
        run: trace,
    },
    CommandSpec {
        id: "image.resample",
        label: "Resample Image",
        menu: "Edit",
        shortcut: None,
        params_doc: "{id?, scale (0.05..4: pixels per original pixel, in each direction)}  — changes the pixel count, not the size in mm; shrinking a picture finer than the layer's line interval makes jobs smaller",
        mutates: true,
        run: resample,
    },
    CommandSpec {
        id: "image.sharpen",
        label: "Sharpen Image",
        menu: "Edit",
        shortcut: None,
        params_doc: "{id?, amount (0..5)}  — unsharp mask applied to the stored picture",
        mutates: true,
        run: sharpen,
    },
];

#[derive(Deserialize)]
struct CropParams {
    id: Option<ShapeId>,
    #[serde(default)]
    left: f64,
    #[serde(default)]
    top: f64,
    #[serde(default)]
    right: f64,
    #[serde(default)]
    bottom: f64,
}

/// Fraction of a side that may be cut off; the opposite sides together must leave something.
const MAX_CROP: f64 = 0.95;

fn crop(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: CropParams = parse(params)?;
    let id = target_image(s, p.id)?;
    let sides = [p.left, p.top, p.right, p.bottom];
    if sides.iter().any(|v| !(v.is_finite() && (0.0..=MAX_CROP).contains(v))) || p.left + p.right >= 1.0 || p.top + p.bottom >= 1.0 {
        return Err(EngineError::BadParams(format!("crop fractions must be between 0 and {MAX_CROP}, and opposite sides must leave some picture")));
    }
    let shape = s.doc.shape_mut(id).ok_or_else(|| EngineError::NotFound(format!("shape {id} does not exist")))?;
    let ShapeKind::Image { image, width_mm, height_mm, .. } = &mut shape.kind else { return Err(not_an_image(id)) };
    let (w, h) = (image.width, image.height);
    let px = |frac: f64, len: u32| (frac * f64::from(len)).round() as u32;
    let (x0, y0) = (px(p.left, w), px(p.top, h));
    let (x1, y1) = (w.saturating_sub(px(p.right, w)), h.saturating_sub(px(p.bottom, h)));
    let cropped = image.crop(x0, y0, x1, y1).ok_or_else(|| EngineError::BadParams("that crop leaves no pixels".into()))?;
    let (mm_per_x, mm_per_y) = (*width_mm / f64::from(w), *height_mm / f64::from(h));
    let (new_w, new_h) = (f64::from(cropped.width) * mm_per_x, f64::from(cropped.height) * mm_per_y);
    let shift = Vec2::new(f64::from(x0) * mm_per_x, f64::from(y0) * mm_per_y);
    *image = Arc::new(cropped);
    *width_mm = new_w;
    *height_mm = new_h;
    // What remains stays exactly where it was on the bed.
    shape.transform *= Affine::translate(shift);
    Ok(json!({ "width_mm": new_w, "height_mm": new_h }))
}

#[derive(Deserialize)]
struct ResampleParams {
    id: Option<ShapeId>,
    scale: f64,
}

fn resample(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: ResampleParams = parse(params)?;
    let id = target_image(s, p.id)?;
    let scale = in_range("scale", p.scale, 0.05, 4.0)?;
    let shape = s.doc.shape_mut(id).ok_or_else(|| EngineError::NotFound(format!("shape {id} does not exist")))?;
    let ShapeKind::Image { image, .. } = &mut shape.kind else { return Err(not_an_image(id)) };
    let scaled = |n: u32| ((f64::from(n) * scale).round() as u32).max(1);
    let new = image
        .resampled(scaled(image.width), scaled(image.height))
        .ok_or_else(|| EngineError::BadParams("that would make the picture too large".into()))?;
    let (w, h) = (new.width, new.height);
    *image = Arc::new(new);
    Ok(json!({ "width_px": w, "height_px": h }))
}

#[derive(Deserialize)]
struct SharpenParams {
    id: Option<ShapeId>,
    amount: f64,
}

fn sharpen(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: SharpenParams = parse(params)?;
    let id = target_image(s, p.id)?;
    let amount = in_range("amount", p.amount, 0.0, 5.0)?;
    let shape = s.doc.shape_mut(id).ok_or_else(|| EngineError::NotFound(format!("shape {id} does not exist")))?;
    let ShapeKind::Image { image, .. } = &mut shape.kind else { return Err(not_an_image(id)) };
    *image = Arc::new(image.sharpened(amount));
    Ok(json!({ "amount": amount }))
}

/// The image a command acts on: the given id, else the only selected shape.
fn target_image(s: &Session, id: Option<ShapeId>) -> Result<ShapeId, EngineError> {
    match id {
        Some(id) => Ok(id),
        None => match s.selection.as_slice() {
            [only] => Ok(*only),
            _ => Err(EngineError::BadParams("select exactly one image, or pass an id".into())),
        },
    }
}

fn not_an_image(id: ShapeId) -> EngineError {
    EngineError::BadParams(format!("shape {id} is not an image"))
}

#[derive(Deserialize)]
struct AdjustParams {
    id: Option<ShapeId>,
    brightness: Option<f64>,
    contrast: Option<f64>,
    gamma: Option<f64>,
    negative: Option<bool>,
    #[serde(default)]
    reset: bool,
}

fn adjust(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: AdjustParams = parse(params)?;
    let id = match p.id {
        Some(id) => id,
        None => match s.selection.as_slice() {
            [only] => *only,
            _ => return Err(EngineError::BadParams("select exactly one image, or pass an id".into())),
        },
    };
    let shape = s.doc.shape_mut(id).ok_or_else(|| EngineError::NotFound(format!("shape {id} does not exist")))?;
    let ShapeKind::Image { adjust, .. } = &mut shape.kind else {
        return Err(EngineError::BadParams(format!("shape {id} is not an image")));
    };
    let mut next = if p.reset { Default::default() } else { *adjust };
    if let Some(v) = p.brightness {
        next.brightness = in_range("brightness", v, -1.0, 1.0)?;
    }
    if let Some(v) = p.contrast {
        next.contrast = in_range("contrast", v, -1.0, 1.0)?;
    }
    if let Some(v) = p.gamma {
        next.gamma = in_range("gamma", v, 0.1, 5.0)?;
    }
    next.negative = p.negative.unwrap_or(next.negative);
    *adjust = next;
    Ok(json!({ "brightness": next.brightness, "contrast": next.contrast, "gamma": next.gamma, "negative": next.negative }))
}

fn in_range(name: &str, v: f64, lo: f64, hi: f64) -> Result<f64, EngineError> {
    if v.is_finite() && (lo..=hi).contains(&v) { Ok(v) } else { Err(EngineError::BadParams(format!("{name} must be between {lo} and {hi}"))) }
}

#[derive(Deserialize)]
struct TraceParams {
    id: Option<ShapeId>,
    #[serde(default = "default_threshold")]
    threshold: f64,
    #[serde(default = "default_tolerance")]
    tolerance: f64,
    #[serde(default = "default_min_area")]
    min_area: f64,
    layer: Option<LayerId>,
    #[serde(default = "yes")]
    keep_image: bool,
    /// Turn gentle bends into curves (corners sharper than 60 degrees stay corners).
    #[serde(default)]
    smooth: bool,
    /// Tonal bands: this many outlines, each around pixels darker than a share of the threshold.
    #[serde(default = "one")]
    levels: u32,
    /// Trace each of this many palette colours separately (needs a coloured picture).
    colors: Option<usize>,
    /// Fit cubic curves through the outlines to within this many pixels (instead of `smooth`).
    fit: Option<f64>,
}

fn one() -> u32 {
    1
}

fn default_threshold() -> f64 {
    128.0
}

fn default_tolerance() -> f64 {
    1.0
}

fn default_min_area() -> f64 {
    4.0
}

fn yes() -> bool {
    true
}

/// Traces each palette colour of a coloured picture as its own shape, on the nearest layer colour.
#[allow(clippy::too_many_arguments)]
fn trace_colors(
    s: &mut Session,
    id: ShapeId,
    colors: usize,
    tolerance: f64,
    min_area: f64,
    smooth: bool,
    fit: Option<f64>,
    keep_image: bool,
) -> Result<Value, EngineError> {
    use laserfoundry_doc::LAYER_COLORS;
    if !(2..=laserfoundry_toolpath::MAX_COLORS).contains(&colors) {
        return Err(EngineError::BadParams(format!("colors must be between 2 and {}", laserfoundry_toolpath::MAX_COLORS)));
    }
    let shape = s.doc.shape(id).ok_or_else(|| EngineError::NotFound(format!("shape {id} does not exist")))?;
    let ShapeKind::Image { image, width_mm, height_mm, .. } = &shape.kind else { return Err(not_an_image(id)) };
    let rgb = image
        .rgb
        .as_ref()
        .ok_or_else(|| EngineError::BadParams("that picture has no colour (grey, or imported by an older version); import it again".into()))?;
    let (w, h) = (image.width as usize, image.height as usize);
    let q = laserfoundry_toolpath::quantize(rgb, colors)
        .ok_or_else(|| EngineError::BadParams("the picture has no ink to trace (it is all paper-white)".into()))?;
    let to_local = Affine::scale_non_uniform(*width_mm / w as f64, *height_mm / h as f64);
    let place = shape.transform * to_local;
    let hex_rgb = |hex: &str| -> [i32; 3] {
        let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
        [((v >> 16) & 255) as i32, ((v >> 8) & 255) as i32, (v & 255) as i32]
    };
    let mut made = Vec::new();
    for (k, colour) in q.palette.iter().enumerate() {
        let label = k as u8;
        let loops = laserfoundry_toolpath::trace(w, h, |x, y| q.labels.get(y * w + x) == Some(&label), tolerance, min_area)
            .ok_or_else(|| EngineError::BadParams("that picture is too large or too detailed to trace".into()))?;
        if loops.is_empty() {
            continue;
        }
        let path = outline_path(&loops, smooth, fit);
        let want = [i32::from(colour[0]), i32::from(colour[1]), i32::from(colour[2])];
        let layer = LAYER_COLORS
            .iter()
            .enumerate()
            .min_by_key(|(_, hex)| hex_rgb(hex).iter().zip(want).map(|(a, b)| (a - b).pow(2)).sum::<i32>())
            .map_or(0, |(i, _)| i as LayerId);
        let mut traced = Shape::path(format!("Traced #{:02x}{:02x}{:02x}", colour[0], colour[1], colour[2]), layer, path);
        traced.transform = place;
        made.push((traced, loops.len()));
    }
    if made.is_empty() {
        return Err(EngineError::BadParams("nothing to trace".into()));
    }
    let outlines: usize = made.iter().map(|(_, n)| n).sum();
    let ids: Vec<ShapeId> = made.into_iter().map(|(shape, _)| s.doc.add_shape(shape)).collect();
    if !keep_image {
        s.doc.shapes.retain(|x| x.id != id);
    }
    s.selection = ids.clone();
    Ok(json!({ "ids": ids, "outlines": outlines, "colors": q.palette.len() }))
}

/// The traced loops as a path: straight steps, gently smoothed, or least-squares fitted curves.
fn outline_path(loops: &[laserfoundry_geom::Polyline], smooth: bool, fit: Option<f64>) -> laserfoundry_geom::BezPath {
    if let Some(tol) = fit {
        let subpaths = loops.iter().map(|l| laserfoundry_geom::fit::fit_polyline(&l.points, l.closed, SMOOTH_CORNER_DEG, tol)).collect();
        laserfoundry_geom::nodes::NodePath { subpaths }.to_bez()
    } else if smooth {
        let subpaths = loops.iter().map(|l| laserfoundry_geom::nodes::smooth_polyline(&l.points, l.closed, SMOOTH_CORNER_DEG)).collect();
        laserfoundry_geom::nodes::NodePath { subpaths }.to_bez()
    } else {
        super::pathops::polylines_to_path(loops)
    }
}

/// Most tonal bands one trace may make.
const MAX_TRACE_LEVELS: u32 = 8;

/// Bends sharper than this stay corners when tracing with `smooth`.
const SMOOTH_CORNER_DEG: f64 = 60.0;

fn trace(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: TraceParams = parse(params)?;
    let id = target_image(s, p.id)?;
    let threshold = in_range("threshold", p.threshold, 0.0, 255.0)?;
    let tolerance = in_range("tolerance", p.tolerance, 0.0, 50.0)?;
    let min_area = in_range("min_area", p.min_area, 0.0, 1e9)?;
    let fit = p.fit.map(|f| in_range("fit", f, 0.05, 50.0)).transpose()?;
    let shape = s.doc.shape(id).ok_or_else(|| EngineError::NotFound(format!("shape {id} does not exist")))?;
    let ShapeKind::Image { image, width_mm, height_mm, adjust } = &shape.kind else { return Err(not_an_image(id)) };
    let (w, h) = (image.width as usize, image.height as usize);
    if let Some(colors) = p.colors {
        return trace_colors(s, id, colors, tolerance, min_area, p.smooth, fit, p.keep_image);
    }
    if !(1..=MAX_TRACE_LEVELS).contains(&p.levels) {
        return Err(EngineError::BadParams(format!("levels must be between 1 and {MAX_TRACE_LEVELS}")));
    }
    let layer = p.layer.unwrap_or(shape.layer);
    if s.doc.layer(layer).is_none() {
        return Err(EngineError::NotFound(format!("layer {layer} does not exist")));
    }
    // Pixel units to the picture's own millimetres, then wherever the picture sits.
    let to_local = Affine::scale_non_uniform(*width_mm / w as f64, *height_mm / h as f64);
    let place = shape.transform * to_local;
    // Lightest band first, so the darkest outlines end up on top.
    let mut traced = Vec::new();
    for k in 1..=p.levels {
        let limit = (threshold * f64::from(k) / f64::from(p.levels)) as f32;
        let loops = laserfoundry_toolpath::trace(
            w,
            h,
            |x, y| image.gray.get(y * w + x).is_some_and(|g| adjust.apply(f32::from(*g)) < limit),
            tolerance,
            min_area,
        )
        .ok_or_else(|| EngineError::BadParams("that picture is too large or too detailed to trace".into()))?;
        if loops.is_empty() {
            continue;
        }
        let path = outline_path(&loops, p.smooth, fit);
        let name = if p.levels == 1 { "Traced".to_string() } else { format!("Traced {k}/{}", p.levels) };
        let mut shape = Shape::path(name, layer, path);
        shape.transform = place;
        traced.push((shape, loops.len()));
    }
    if traced.is_empty() {
        return Err(EngineError::BadParams("nothing in the picture is darker than the threshold".into()));
    }
    let count: usize = traced.iter().map(|(_, n)| n).sum();
    let ids: Vec<ShapeId> = traced.into_iter().map(|(shape, _)| s.doc.add_shape(shape)).collect();
    if !p.keep_image {
        s.doc.shapes.retain(|x| x.id != id);
    }
    s.selection = ids.clone();
    Ok(json!({ "id": ids.last(), "ids": ids, "outlines": count }))
}
