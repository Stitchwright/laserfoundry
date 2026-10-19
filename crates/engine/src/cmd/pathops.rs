use laserfoundry_doc::{Shape, ShapeId};
use laserfoundry_geom::{BezPath, Polyline};
use laserfoundry_pathops::{BoolOp, Join, boolean, join, offset};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, id_list, parse, shapes::finite};
use crate::{EngineError, Session};

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "path.boolean",
        label: "Boolean",
        menu: "Arrange/Boolean",
        shortcut: None,
        params_doc: "{op: union|difference|intersect|xor, ids? (default: selection, order matters: difference is the first minus the rest), keep_originals?: false}  — closed shapes only",
        mutates: true,
        run: boolean_op,
    },
    CommandSpec {
        id: "path.join",
        label: "Join Paths",
        menu: "Arrange",
        shortcut: None,
        params_doc: "{ids?, tolerance? (mm, default 0.05), keep_originals?: false}  — chains open lines whose ends meet into longer paths, closing loops; one result per chain, on the source layer",
        mutates: true,
        run: join_op,
    },
    CommandSpec {
        id: "path.offset",
        label: "Offset",
        menu: "Arrange",
        shortcut: None,
        params_doc: "{ids?, distance (mm; negative shrinks closed shapes), join?: round|miter|bevel, keep_originals?: true}  — open paths become an outline around the line",
        mutates: true,
        run: offset_op,
    },
];

#[derive(Deserialize)]
struct BooleanParams {
    op: Op,
    ids: Option<Vec<ShapeId>>,
    #[serde(default)]
    keep_originals: bool,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Op {
    Union,
    Difference,
    Intersect,
    Xor,
}

fn boolean_op(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: BooleanParams = parse(params)?;
    let ids = existing(s, &id_list(&p.ids, &s.selection));
    if ids.len() < 2 {
        return Err(EngineError::BadParams("a boolean needs at least two shapes".into()));
    }
    let op = match p.op {
        Op::Union => BoolOp::Union,
        Op::Difference => BoolOp::Difference,
        Op::Intersect => BoolOp::Intersect,
        Op::Xor => BoolOp::Xor,
    };
    let mut regions = Vec::new();
    for id in &ids {
        regions.push(closed_region(s, *id)?);
    }
    let mut it = regions.into_iter();
    let mut result = it.next().unwrap_or_default();
    for next in it {
        result = boolean(&result, &next, op).map_err(|e| EngineError::BadParams(e.to_string()))?;
    }
    if result.is_empty() {
        return Err(EngineError::BadParams("the result is empty (the shapes do not overlap that way)".into()));
    }
    let first = s.doc.shape(ids[0]).map(|sh| sh.layer).unwrap_or(0);
    let name = match p.op {
        Op::Union => "Union",
        Op::Difference => "Difference",
        Op::Intersect => "Intersection",
        Op::Xor => "Exclusive or",
    };
    let new_id = s.doc.add_shape(Shape::path(name, first, polylines_to_path(&result)));
    if !p.keep_originals {
        s.doc.remove_shapes(&ids);
    }
    s.selection = vec![new_id];
    Ok(json!({ "id": new_id, "contours": result.len() }))
}

#[derive(Deserialize)]
struct OffsetParams {
    ids: Option<Vec<ShapeId>>,
    distance: f64,
    #[serde(default)]
    join: JoinParam,
    #[serde(default = "yes")]
    keep_originals: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
enum JoinParam {
    #[default]
    Round,
    Miter,
    Bevel,
}

fn offset_op(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: OffsetParams = parse(params)?;
    finite(&[p.distance])?;
    let ids = existing(s, &id_list(&p.ids, &s.selection));
    if ids.is_empty() {
        return Err(EngineError::NothingSelected);
    }
    let join = match p.join {
        JoinParam::Round => Join::Round,
        JoinParam::Miter => Join::Miter,
        JoinParam::Bevel => Join::Bevel,
    };
    let mut created = Vec::new();
    let mut vanished = 0usize;
    for id in &ids {
        let Some(shape) = s.doc.shape(*id) else { continue };
        if shape.vector_path().is_none() {
            return Err(EngineError::BadParams(format!("{} is an image and cannot be offset", shape.name)));
        }
        let (layer, name) = (shape.layer, format!("{} offset", shape.name));
        let result = offset(&shape.world_polylines(), p.distance, join).map_err(|e| EngineError::BadParams(e.to_string()))?;
        if result.is_empty() {
            vanished += 1;
            continue;
        }
        created.push(s.doc.add_shape(Shape::path(name, layer, polylines_to_path(&result))));
    }
    if created.is_empty() {
        return Err(EngineError::BadParams("the offset leaves nothing (the shapes collapse at that distance)".into()));
    }
    if !p.keep_originals {
        s.doc.remove_shapes(&ids);
    }
    s.selection = created.clone();
    Ok(json!({ "ids": created, "vanished": vanished }))
}

#[derive(Deserialize)]
struct JoinParams {
    ids: Option<Vec<ShapeId>>,
    #[serde(default = "default_join_tolerance")]
    tolerance: f64,
    #[serde(default)]
    keep_originals: bool,
}

/// Default gap bridged when joining, mm.
pub(super) const DEFAULT_JOIN_TOLERANCE: f64 = 0.05;

fn default_join_tolerance() -> f64 {
    DEFAULT_JOIN_TOLERANCE
}

fn join_op(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: JoinParams = parse(params)?;
    finite(&[p.tolerance])?;
    let ids = existing(s, &id_list(&p.ids, &s.selection));
    if ids.is_empty() {
        return Err(EngineError::NothingSelected);
    }
    // Join within each layer, so cut settings stay with the geometry.
    let mut by_layer: Vec<(u32, Vec<Polyline>)> = Vec::new();
    for id in &ids {
        let Some(shape) = s.doc.shape(*id) else { continue };
        if shape.vector_path().is_none() {
            return Err(EngineError::BadParams(format!("{} is an image and cannot be joined", shape.name)));
        }
        let lines = shape.world_polylines();
        match by_layer.iter_mut().find(|(l, _)| *l == shape.layer) {
            Some((_, v)) => v.extend(lines),
            None => by_layer.push((shape.layer, lines)),
        }
    }
    let mut created = Vec::new();
    let (mut closed, mut open) = (0usize, 0usize);
    for (layer, lines) in by_layer {
        for chain in join(&lines, p.tolerance).map_err(|e| EngineError::BadParams(e.to_string()))? {
            if chain.closed {
                closed += 1;
            } else {
                open += 1;
            }
            created.push(s.doc.add_shape(Shape::path(
                if chain.closed { "Joined shape" } else { "Joined path" },
                layer,
                polylines_to_path(std::slice::from_ref(&chain)),
            )));
        }
    }
    if !p.keep_originals {
        s.doc.remove_shapes(&ids);
    }
    s.selection = created.clone();
    Ok(json!({ "ids": created, "closed": closed, "open": open }))
}

fn existing(s: &Session, ids: &[ShapeId]) -> Vec<ShapeId> {
    ids.iter().copied().filter(|id| s.doc.shape(*id).is_some()).collect()
}

fn closed_region(s: &Session, id: ShapeId) -> Result<Vec<Polyline>, EngineError> {
    let shape = s.doc.shape(id).ok_or_else(|| EngineError::NotFound(format!("shape {id} does not exist")))?;
    if shape.vector_path().is_none() {
        return Err(EngineError::BadParams(format!("{} is an image; booleans need vector shapes", shape.name)));
    }
    let closed: Vec<Polyline> = shape.world_polylines().into_iter().filter(|l| l.closed).collect();
    if closed.is_empty() {
        return Err(EngineError::BadParams(format!("{} has no closed outline (open lines and stroke text cannot be combined)", shape.name)));
    }
    Ok(closed)
}

pub(super) fn polylines_to_path(lines: &[Polyline]) -> BezPath {
    let mut path = BezPath::new();
    for l in lines {
        let Some((first, rest)) = l.points.split_first() else { continue };
        path.move_to(*first);
        for p in rest {
            path.line_to(*p);
        }
        if l.closed {
            path.close_path();
        }
    }
    path
}
