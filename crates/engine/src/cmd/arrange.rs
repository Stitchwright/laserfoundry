use laserfoundry_doc::ShapeId;
use laserfoundry_geom::{Affine, Point};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, id_list, parse, shapes::finite};
use crate::{EngineError, Session};

/// Most copies one array command may create.
pub const MAX_COPIES: u32 = 10_000;
/// Most shapes one array may add (the copies times the shapes being copied).
pub const MAX_NEW_SHAPES: u64 = 200_000;

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "arrange.grid",
        label: "Grid Array",
        menu: "Arrange",
        shortcut: None,
        params_doc: "{ids?, columns, rows, gap_x? (mm between copies, default 5), gap_y?}  — copies the selection into a grid; the original stays at the top-left",
        mutates: true,
        run: grid,
    },
    CommandSpec {
        id: "arrange.circular",
        label: "Circular Array",
        menu: "Arrange",
        shortcut: None,
        params_doc: "{ids?, count, radius? (selection centre to array centre, default 30 mm), direction? (degrees toward the array centre, y down: 90 = below, default 90), angle? (total sweep, default 360)}  — copies rotate about the array centre",
        mutates: true,
        run: circular,
    },
];

#[derive(Deserialize)]
struct GridParams {
    ids: Option<Vec<ShapeId>>,
    columns: u32,
    rows: u32,
    #[serde(default = "default_gap")]
    gap_x: f64,
    #[serde(default = "default_gap")]
    gap_y: f64,
}

fn default_gap() -> f64 {
    5.0
}

fn grid(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: GridParams = parse(params)?;
    finite(&[p.gap_x, p.gap_y])?;
    check_count(u64::from(p.columns).saturating_mul(u64::from(p.rows)))?;
    let ids = selected(s, &p.ids)?;
    check_total(ids.len(), u64::from(p.columns).saturating_mul(u64::from(p.rows)))?;
    let b = s.doc.bounds_of(&ids).ok_or(EngineError::NothingSelected)?;
    let (pitch_x, pitch_y) = (b.width() + p.gap_x, b.height() + p.gap_y);
    let mut all = ids.clone();
    for row in 0..p.rows {
        for col in 0..p.columns {
            if row == 0 && col == 0 {
                continue;
            }
            let m = Affine::translate((f64::from(col) * pitch_x, f64::from(row) * pitch_y));
            all.extend(copy_all(s, &ids, m));
        }
    }
    s.selection = all.clone();
    Ok(json!({ "count": all.len() / ids.len().max(1), "ids": all }))
}

#[derive(Deserialize)]
struct CircularParams {
    ids: Option<Vec<ShapeId>>,
    count: u32,
    #[serde(default = "default_radius")]
    radius: f64,
    #[serde(default = "default_direction")]
    direction: f64,
    #[serde(default = "default_sweep")]
    angle: f64,
}

fn default_radius() -> f64 {
    30.0
}

fn default_direction() -> f64 {
    90.0
}

fn default_sweep() -> f64 {
    360.0
}

fn circular(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: CircularParams = parse(params)?;
    finite(&[p.radius, p.direction, p.angle])?;
    check_count(u64::from(p.count))?;
    let ids = selected(s, &p.ids)?;
    check_total(ids.len(), u64::from(p.count))?;
    let b = s.doc.bounds_of(&ids).ok_or(EngineError::NothingSelected)?;
    let (c, dir) = (b.center(), p.direction.to_radians());
    let centre = Point::new(c.x + p.radius * dir.cos(), c.y + p.radius * dir.sin());
    // A full circle spreads the copies evenly with no copy on top of the original; an arc runs from
    // the original to the end of the sweep.
    let step =
        if (p.angle.abs() - 360.0).abs() < 1e-9 { p.angle / f64::from(p.count) } else { p.angle / f64::from(p.count.saturating_sub(1).max(1)) };
    let mut all = ids.clone();
    for k in 1..p.count {
        let m = Affine::rotate_about((step * f64::from(k)).to_radians(), centre);
        all.extend(copy_all(s, &ids, m));
    }
    s.selection = all.clone();
    Ok(json!({ "count": p.count, "ids": all }))
}

fn check_count(n: u64) -> Result<(), EngineError> {
    if (1..=u64::from(MAX_COPIES)).contains(&n) {
        Ok(())
    } else {
        Err(EngineError::BadParams(format!("an array must make between 1 and {MAX_COPIES} copies")))
    }
}

/// Refuses an array that would add more than [`MAX_NEW_SHAPES`] shapes.
fn check_total(shapes: usize, copies: u64) -> Result<(), EngineError> {
    if (shapes as u64).saturating_mul(copies) > MAX_NEW_SHAPES {
        return Err(EngineError::BadParams(format!("an array may add at most {MAX_NEW_SHAPES} shapes")));
    }
    Ok(())
}

fn selected(s: &Session, ids: &Option<Vec<ShapeId>>) -> Result<Vec<ShapeId>, EngineError> {
    let ids: Vec<ShapeId> = id_list(ids, &s.selection).into_iter().filter(|id| s.doc.shape(*id).is_some()).collect();
    if ids.is_empty() { Err(EngineError::NothingSelected) } else { Ok(ids) }
}

/// Duplicates every shape in `ids` with `m` applied on top of its transform (groups stay groups).
fn copy_all(s: &mut Session, ids: &[ShapeId], m: Affine) -> Vec<ShapeId> {
    s.doc.duplicate_shapes(ids, m)
}
