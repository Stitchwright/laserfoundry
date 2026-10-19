use std::f64::consts::TAU;

use laserfoundry_doc::{LayerId, Shape};
use laserfoundry_geom::{BezPath, Point, ellipse_path, line_path, rect_path, rounded_rect_path};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, parse};
use crate::{EngineError, Session};

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "shape.rectangle",
        label: "Rectangle",
        menu: "Create",
        shortcut: Some("R"),
        params_doc: "{x, y, width, height, radius? (corner radius), layer?}  (mm)",
        mutates: true,
        run: rectangle,
    },
    CommandSpec {
        id: "shape.ellipse",
        label: "Ellipse",
        menu: "Create",
        shortcut: Some("E"),
        params_doc: "{cx, cy, rx, ry, layer?}  (mm)",
        mutates: true,
        run: ellipse,
    },
    CommandSpec {
        id: "shape.line",
        label: "Line",
        menu: "Create",
        shortcut: Some("L"),
        params_doc: "{x1, y1, x2, y2, layer?}  (mm)",
        mutates: true,
        run: line,
    },
    CommandSpec {
        id: "shape.polygon",
        label: "Regular Polygon",
        menu: "Create",
        shortcut: None,
        params_doc: "{cx, cy, radius, sides (3..=100), layer?}  (mm)",
        mutates: true,
        run: polygon,
    },
    CommandSpec {
        id: "shape.star",
        label: "Star",
        menu: "Create",
        shortcut: None,
        params_doc: "{cx, cy, outer_radius, inner_radius (< outer), points (3..=100), layer?}  (mm)",
        mutates: true,
        run: star,
    },
];

#[derive(Deserialize)]
struct RectParams {
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    width: f64,
    height: f64,
    /// Corner radius in mm (clamped to half the shorter side).
    #[serde(default)]
    radius: f64,
    layer: Option<LayerId>,
}

fn rectangle(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: RectParams = parse(params)?;
    finite(&[p.x, p.y, p.width, p.height, p.radius])?;
    if p.width <= 0.0 || p.height <= 0.0 {
        return Err(EngineError::BadParams("width and height must be positive".into()));
    }
    let path = if p.radius > 0.0 { rounded_rect_path(p.x, p.y, p.width, p.height, p.radius) } else { rect_path(p.x, p.y, p.width, p.height) };
    add(s, if p.radius > 0.0 { "Rounded rectangle" } else { "Rectangle" }, p.layer, path)
}

#[derive(Deserialize)]
struct EllipseParams {
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    layer: Option<LayerId>,
}

fn ellipse(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: EllipseParams = parse(params)?;
    finite(&[p.cx, p.cy, p.rx, p.ry])?;
    if p.rx <= 0.0 || p.ry <= 0.0 {
        return Err(EngineError::BadParams("radii must be positive".into()));
    }
    add(s, "Ellipse", p.layer, ellipse_path(p.cx, p.cy, p.rx, p.ry))
}

#[derive(Deserialize)]
struct LineParams {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    layer: Option<LayerId>,
}

fn line(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: LineParams = parse(params)?;
    finite(&[p.x1, p.y1, p.x2, p.y2])?;
    add(s, "Line", p.layer, line_path(Point::new(p.x1, p.y1), Point::new(p.x2, p.y2)))
}

#[derive(Deserialize)]
struct PolygonParams {
    cx: f64,
    cy: f64,
    radius: f64,
    sides: u32,
    layer: Option<LayerId>,
}

fn polygon(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: PolygonParams = parse(params)?;
    finite(&[p.cx, p.cy, p.radius])?;
    if p.radius <= 0.0 || !(3..=100).contains(&p.sides) {
        return Err(EngineError::BadParams("radius must be positive and sides between 3 and 100".into()));
    }
    let mut path = BezPath::new();
    for i in 0..p.sides {
        // Start at the top so a triangle points up.
        let a = TAU * f64::from(i) / f64::from(p.sides) - TAU / 4.0;
        let pt = (p.cx + p.radius * a.cos(), p.cy + p.radius * a.sin());
        if i == 0 {
            path.move_to(pt);
        } else {
            path.line_to(pt);
        }
    }
    path.close_path();
    add(s, "Polygon", p.layer, path)
}

#[derive(Deserialize)]
struct StarParams {
    cx: f64,
    cy: f64,
    outer_radius: f64,
    inner_radius: f64,
    points: u32,
    layer: Option<LayerId>,
}

fn star(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: StarParams = parse(params)?;
    finite(&[p.cx, p.cy, p.outer_radius, p.inner_radius])?;
    if !(p.inner_radius > 0.0 && p.outer_radius > p.inner_radius) || !(3..=100).contains(&p.points) {
        return Err(EngineError::BadParams("radii must satisfy 0 < inner < outer and points must be between 3 and 100".into()));
    }
    let mut path = BezPath::new();
    for i in 0..p.points * 2 {
        // One point straight up; tips on even steps, the notches between them on odd ones.
        let a = TAU * f64::from(i) / f64::from(p.points * 2) - TAU / 4.0;
        let r = if i % 2 == 0 { p.outer_radius } else { p.inner_radius };
        let pt = (p.cx + r * a.cos(), p.cy + r * a.sin());
        if i == 0 {
            path.move_to(pt);
        } else {
            path.line_to(pt);
        }
    }
    path.close_path();
    add(s, "Star", p.layer, path)
}

fn add(s: &mut Session, name: &str, layer: Option<LayerId>, path: BezPath) -> Result<Value, EngineError> {
    let id = s.doc.add_shape(Shape::path(name, layer.unwrap_or(0), path));
    s.selection = vec![id];
    Ok(json!({ "id": id }))
}

pub(super) fn finite(values: &[f64]) -> Result<(), EngineError> {
    if values.iter().all(|v| v.is_finite() && v.abs() < 1e7) {
        Ok(())
    } else {
        Err(EngineError::BadParams("numbers must be finite and within ±10,000,000 mm".into()))
    }
}
