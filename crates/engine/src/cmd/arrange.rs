use std::collections::HashMap;

use laserfoundry_doc::{GroupId, ShapeId};
use laserfoundry_geom::{Affine, Point, Rect};
use laserfoundry_nest::{Options, Part};
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
    CommandSpec {
        id: "arrange.nest",
        label: "Nest",
        menu: "Arrange",
        shortcut: None,
        params_doc: "{ids?, x?, y?, width?, height? (the region in mm; default the whole bed), gap? (mm between parts, default 2), margin? (mm kept free at the region's edge, default 0), rotations? (evenly spaced turns tried for each part, 1 keeps their angles, default 4), resolution? (cell size in mm; default chosen from the region)}  — packs the selection into the region without overlaps, a group moving as one part; parts that do not fit stay where they were, are listed in `unplaced` and become the selection. Other shapes on the bed are ignored",
        mutates: true,
        run: nest,
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

#[derive(Deserialize)]
struct NestParams {
    ids: Option<Vec<ShapeId>>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    #[serde(default = "default_nest_gap")]
    gap: f64,
    #[serde(default)]
    margin: f64,
    #[serde(default = "default_rotations")]
    rotations: u32,
    resolution: Option<f64>,
}

fn default_nest_gap() -> f64 {
    2.0
}

fn default_rotations() -> u32 {
    4
}

fn nest(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: NestParams = parse(params)?;
    let bed = (s.doc.machine.bed_width, s.doc.machine.bed_height);
    let (x, y) = (p.x.unwrap_or(0.0), p.y.unwrap_or(0.0));
    let (width, height) = (p.width.unwrap_or(bed.0), p.height.unwrap_or(bed.1));
    finite(&[x, y, width, height, p.gap, p.margin, p.resolution.unwrap_or(1.0)])?;
    let ids = s.doc.expand_groups(&selected(s, &p.ids)?);

    // A group is one part; every other shape is a part of its own.
    let mut units: Vec<Vec<ShapeId>> = Vec::new();
    let mut by_group: HashMap<GroupId, usize> = HashMap::new();
    for id in ids {
        let Some(group) = s.doc.shape(id).map(|shape| shape.group) else { continue };
        match group.map(|g| (g, by_group.get(&g).copied())) {
            Some((_, Some(unit))) => {
                if let Some(members) = units.get_mut(unit) {
                    members.push(id);
                }
            }
            Some((g, None)) => {
                by_group.insert(g, units.len());
                units.push(vec![id]);
            }
            None => units.push(vec![id]),
        }
    }
    let parts: Vec<Part> = units
        .iter()
        .map(|members| Part { outlines: members.iter().filter_map(|id| s.doc.shape(*id)).flat_map(|shape| shape.world_polylines()).collect() })
        .collect();

    let options = Options { gap: p.gap, margin: p.margin, rotations: p.rotations, resolution: p.resolution };
    let result =
        laserfoundry_nest::nest(&parts, Rect::new(x, y, x + width, y + height), &options).map_err(|e| EngineError::BadParams(e.to_string()))?;

    let mut placed_ids = Vec::new();
    for placement in &result.placed {
        let m = placement.affine();
        for id in units.get(placement.part).into_iter().flatten() {
            if let Some(shape) = s.doc.shape_mut(*id) {
                shape.transform = m * shape.transform;
                placed_ids.push(*id);
            }
        }
    }
    let unplaced_ids: Vec<ShapeId> = result.unplaced.iter().filter_map(|i| units.get(*i)).flatten().copied().collect();
    s.selection = if unplaced_ids.is_empty() { placed_ids.clone() } else { unplaced_ids.clone() };
    Ok(json!({
        "placed": result.placed.len(),
        "unplaced": unplaced_ids,
        "ids": placed_ids,
        "cell": result.cell,
        "stopped_early": result.stopped_early,
    }))
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
