use laserfoundry_doc::{CutOverride, LAYER_COLORS, LayerId, Shape, ShapeId, ShapeKind, TextSettings};
use laserfoundry_geom::{Affine, rect_path};
use serde::Deserialize;
use serde_json::{Value, json};

use super::text::generate;
use super::{CommandSpec, id_list, parse, shapes::finite};
use crate::{EngineError, Session};

/// Most cells a test grid may have.
pub const MAX_CELLS: u32 = 400;

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "shape.cut_override",
        label: "Cut Settings Override",
        menu: "Laser",
        shortcut: None,
        params_doc: "{ids?, speed? (mm/min), min_power?, max_power? (%), passes?, clear?: bool}  — replaces the layer's settings for these shapes only; clear removes the override",
        mutates: true,
        run: cut_override,
    },
    CommandSpec {
        id: "shape.set_order",
        label: "Cut Order",
        menu: "Laser",
        shortcut: None,
        params_doc: "{ids?, order: integer -1000..1000}  — cut priority within the layer: lower numbers are cut first; shapes default to 0 and ties keep the drawing order",
        mutates: true,
        run: set_order,
    },
    CommandSpec {
        id: "generate.test_grid",
        label: "Material Test Grid",
        menu: "Laser",
        shortcut: None,
        params_doc: "{cols? (speed steps, default 5), rows? (power steps, default 5), speed_min, speed_max (mm/min), power_min, power_max (%), size? (cell mm, default 10), gap? (default 2), x?, y? (top-left of the grid, default 20,20), layer? (default 0), labels?: true, label_layer?}  — one square per speed × power, each with its own settings; speeds across, power down",
        mutates: true,
        run: test_grid,
    },
];

#[derive(Deserialize)]
struct OverrideParams {
    ids: Option<Vec<ShapeId>>,
    speed: Option<f64>,
    min_power: Option<f64>,
    max_power: Option<f64>,
    passes: Option<u32>,
    #[serde(default)]
    clear: bool,
}

fn cut_override(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: OverrideParams = parse(params)?;
    let ids: Vec<ShapeId> = id_list(&p.ids, &s.selection).into_iter().filter(|id| s.doc.shape(*id).is_some()).collect();
    if ids.is_empty() {
        return Err(EngineError::NothingSelected);
    }
    let check = |name: &str, v: Option<f64>, lo: f64, hi: f64| -> Result<Option<f64>, EngineError> {
        match v {
            Some(v) if !(v.is_finite() && (lo..=hi).contains(&v)) => Err(EngineError::BadParams(format!("{name} must be between {lo} and {hi}"))),
            other => Ok(other),
        }
    };
    let (speed, min_power, max_power) =
        (check("speed", p.speed, 1.0, 100_000.0)?, check("min_power", p.min_power, 0.0, 100.0)?, check("max_power", p.max_power, 0.0, 100.0)?);
    for id in &ids {
        let Some(shape) = s.doc.shape_mut(*id) else { continue };
        if p.clear {
            shape.cut = None;
            continue;
        }
        let mut c = shape.cut.unwrap_or_default();
        c.speed = speed.or(c.speed);
        c.min_power = min_power.or(c.min_power);
        c.max_power = max_power.or(c.max_power);
        c.passes = p.passes.map(|n| n.clamp(1, 1000)).or(c.passes);
        shape.cut = Some(c).filter(|c| !c.is_empty());
    }
    Ok(json!({ "shapes": ids.len() }))
}

#[derive(Deserialize)]
struct OrderParams {
    ids: Option<Vec<ShapeId>>,
    order: i32,
}

fn set_order(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: OrderParams = parse(params)?;
    if !(-1000..=1000).contains(&p.order) {
        return Err(EngineError::BadParams("order must be between -1000 and 1000".into()));
    }
    let ids: Vec<ShapeId> = id_list(&p.ids, &s.selection).into_iter().filter(|id| s.doc.shape(*id).is_some()).collect();
    if ids.is_empty() {
        return Err(EngineError::NothingSelected);
    }
    for id in &ids {
        if let Some(shape) = s.doc.shape_mut(*id) {
            shape.order = p.order;
        }
    }
    Ok(json!({ "shapes": ids.len(), "order": p.order }))
}

#[derive(Deserialize)]
struct GridParams {
    #[serde(default = "five")]
    cols: u32,
    #[serde(default = "five")]
    rows: u32,
    speed_min: Option<f64>,
    speed_max: Option<f64>,
    power_min: Option<f64>,
    power_max: Option<f64>,
    #[serde(default = "ten")]
    size: f64,
    #[serde(default = "two")]
    gap: f64,
    #[serde(default = "twenty")]
    x: f64,
    #[serde(default = "twenty")]
    y: f64,
    #[serde(default)]
    layer: LayerId,
    #[serde(default = "yes")]
    labels: bool,
    label_layer: Option<LayerId>,
}

fn five() -> u32 {
    5
}
fn ten() -> f64 {
    10.0
}
fn two() -> f64 {
    2.0
}
fn twenty() -> f64 {
    20.0
}
fn yes() -> bool {
    true
}

/// Height of the label text, mm.
const LABEL_HEIGHT: f64 = 2.5;

fn test_grid(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: GridParams = parse(params)?;
    finite(&[p.size, p.gap, p.x, p.y])?;
    if !(1..=MAX_COLS_ROWS).contains(&p.cols) || !(1..=MAX_COLS_ROWS).contains(&p.rows) || p.cols * p.rows > MAX_CELLS {
        return Err(EngineError::BadParams(format!("a test grid has 1 to {MAX_COLS_ROWS} columns and rows and at most {MAX_CELLS} cells")));
    }
    if !(0.5..=200.0).contains(&p.size) || !(0.0..=100.0).contains(&p.gap) {
        return Err(EngineError::BadParams("cell size must be 0.5-200 mm and the gap 0-100 mm".into()));
    }
    let layer = s.doc.layer(p.layer).cloned().ok_or_else(|| EngineError::NotFound(format!("layer {} does not exist", p.layer)))?;
    let range = |name: &str, lo: Option<f64>, hi: Option<f64>, default: (f64, f64), min: f64, max: f64| -> Result<(f64, f64), EngineError> {
        let (lo, hi) = (lo.unwrap_or(default.0), hi.unwrap_or(default.1));
        if [lo, hi].iter().all(|v| v.is_finite() && (min..=max).contains(v)) {
            Ok((lo, hi))
        } else {
            Err(EngineError::BadParams(format!("{name} must be within {min}-{max}")))
        }
    };
    let (speed_lo, speed_hi) = range("speed", p.speed_min, p.speed_max, (layer.speed * 0.4, layer.speed * 1.6), 1.0, 100_000.0)?;
    let (power_lo, power_hi) = range("power", p.power_min, p.power_max, (layer.max_power * 0.2, layer.max_power), 0.0, 100.0)?;
    let step = |lo: f64, hi: f64, i: u32, n: u32| if n <= 1 { lo } else { lo + (hi - lo) * f64::from(i) / f64::from(n - 1) };
    let pitch = p.size + p.gap;

    let mut ids = Vec::new();
    for r in 0..p.rows {
        for c in 0..p.cols {
            let (speed, power) = (step(speed_lo, speed_hi, c, p.cols).round(), step(power_lo, power_hi, r, p.rows).round());
            let mut shape = Shape::path(format!("Test {speed:.0} mm/min {power:.0}%"), p.layer, rect_path(0.0, 0.0, p.size, p.size));
            shape.transform = Affine::translate((p.x + f64::from(c) * pitch, p.y + f64::from(r) * pitch));
            shape.cut = Some(CutOverride { speed: Some(speed), max_power: Some(power), min_power: Some(layer.min_power.min(power)), passes: None });
            ids.push(s.doc.add_shape(shape));
        }
    }
    if p.labels {
        let label_layer = p.label_layer.unwrap_or((p.layer + 1) % LAYER_COLORS.len() as LayerId);
        if s.doc.layer(label_layer).is_none() {
            return Err(EngineError::NotFound(format!("layer {label_layer} does not exist")));
        }
        let style = TextSettings { height: LABEL_HEIGHT, ..TextSettings::default() };
        let mut label = |s: &mut Session, text: String, x: f64, y: f64| -> Result<(), EngineError> {
            let (path, _) = generate(&text, &style)?;
            let mut shape = Shape::path(text.clone(), label_layer, path.clone());
            shape.kind = ShapeKind::Text { content: text, style: style.clone(), path };
            shape.transform = Affine::translate((x, y));
            ids.push(s.doc.add_shape(shape));
            Ok(())
        };
        for c in 0..p.cols {
            let x = p.x + f64::from(c) * pitch;
            label(s, format!("{:.0}", step(speed_lo, speed_hi, c, p.cols)), x, p.y - LABEL_HEIGHT - 1.5)?;
        }
        for r in 0..p.rows {
            let y = p.y + f64::from(r) * pitch + (p.size - LABEL_HEIGHT) / 2.0;
            label(s, format!("{:.0}%", step(power_lo, power_hi, r, p.rows)), p.x - 10.0, y)?;
        }
    }
    s.selection = ids.clone();
    Ok(json!({ "cells": p.cols * p.rows, "ids": ids }))
}

const MAX_COLS_ROWS: u32 = 20;
