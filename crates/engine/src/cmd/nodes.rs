//! Node editing: commands that reshape a path by its nodes and handles. Points arrive in
//! workspace mm and are mapped through the shape's transform.

use laserfoundry_doc::{LayerId, Shape, ShapeId, ShapeKind};
use laserfoundry_geom::nodes::{MAX_NODES, Node, NodeError, NodePath, Side};
use laserfoundry_geom::{Affine, BezPath, Point};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, id_list, parse, shapes::finite};
use crate::{EngineError, Session};

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "node.move",
        label: "Move Node",
        menu: "",
        shortcut: None,
        params_doc: "{id, subpath?, index, x, y}  — workspace mm; the node's handles go with it",
        mutates: true,
        run: move_node,
    },
    CommandSpec {
        id: "node.move_handle",
        label: "Move Handle",
        menu: "",
        shortcut: None,
        params_doc: "{id, subpath?, index, side: in|out, x, y}  — the handle's tip, workspace mm; a smooth node keeps its other handle in line",
        mutates: true,
        run: move_handle,
    },
    CommandSpec {
        id: "node.insert",
        label: "Insert Node",
        menu: "",
        shortcut: None,
        params_doc: "{id, subpath?, segment, t? (0..1, default 0.5)}  — adds a node without changing the shape",
        mutates: true,
        run: insert,
    },
    CommandSpec {
        id: "node.delete",
        label: "Delete Node",
        menu: "",
        shortcut: None,
        params_doc: "{id, subpath?, index}",
        mutates: true,
        run: delete,
    },
    CommandSpec {
        id: "node.set_smooth",
        label: "Smooth / Corner Node",
        menu: "",
        shortcut: None,
        params_doc: "{id, subpath?, index, smooth: bool}",
        mutates: true,
        run: set_smooth,
    },
    CommandSpec {
        id: "node.set_segment",
        label: "Line / Curve Segment",
        menu: "",
        shortcut: None,
        params_doc: "{id, subpath?, segment, curve: bool}  — the segment from node `segment` to the next",
        mutates: true,
        run: set_segment,
    },
    CommandSpec {
        id: "node.set_closed",
        label: "Close / Open Path",
        menu: "",
        shortcut: None,
        params_doc: "{id, subpath?, closed: bool}",
        mutates: true,
        run: set_closed,
    },
    CommandSpec {
        id: "path.create",
        label: "Create Path",
        menu: "",
        shortcut: None,
        params_doc: "{nodes: [{x, y, in?: [x, y], out?: [x, y], smooth?}], closed?, layer?}  — workspace mm; handle tips are absolute points. Used by the pen tool",
        mutates: true,
        run: create,
    },
    CommandSpec {
        id: "edit.convert_to_path",
        label: "Convert to Path",
        menu: "Arrange",
        shortcut: None,
        params_doc: "{ids?}  — turns text into an ordinary editable path",
        mutates: true,
        run: convert_to_path,
    },
];

#[derive(Deserialize, Clone, Copy)]
struct At {
    id: ShapeId,
    #[serde(default)]
    subpath: usize,
}

#[derive(Deserialize)]
struct MoveParams {
    #[serde(flatten)]
    at: At,
    index: usize,
    x: f64,
    y: f64,
}

fn move_node(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: MoveParams = parse(params)?;
    finite(&[p.x, p.y])?;
    edit(s, p.at, |np, to_local| np.move_node(p.at.subpath, p.index, to_local * Point::new(p.x, p.y)))
}

#[derive(Deserialize)]
struct HandleParams {
    #[serde(flatten)]
    at: At,
    index: usize,
    side: HandleSide,
    x: f64,
    y: f64,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum HandleSide {
    In,
    Out,
}

fn move_handle(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: HandleParams = parse(params)?;
    finite(&[p.x, p.y])?;
    let side = match p.side {
        HandleSide::In => Side::In,
        HandleSide::Out => Side::Out,
    };
    edit(s, p.at, |np, to_local| np.move_handle(p.at.subpath, p.index, side, to_local * Point::new(p.x, p.y)))
}

#[derive(Deserialize)]
struct InsertParams {
    #[serde(flatten)]
    at: At,
    segment: usize,
    #[serde(default = "half")]
    t: f64,
}

fn half() -> f64 {
    0.5
}

fn insert(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: InsertParams = parse(params)?;
    finite(&[p.t])?;
    let mut new_index = 0;
    let out = edit(s, p.at, |np, _| {
        new_index = np.insert(p.at.subpath, p.segment, p.t)?;
        Ok(())
    })?;
    Ok(json!({ "index": new_index, "id": out["id"] }))
}

#[derive(Deserialize)]
struct IndexParams {
    #[serde(flatten)]
    at: At,
    index: usize,
}

fn delete(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: IndexParams = parse(params)?;
    edit(s, p.at, |np, _| np.delete(p.at.subpath, p.index))
}

#[derive(Deserialize)]
struct SmoothParams {
    #[serde(flatten)]
    at: At,
    index: usize,
    smooth: bool,
}

fn set_smooth(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: SmoothParams = parse(params)?;
    edit(s, p.at, |np, _| np.set_smooth(p.at.subpath, p.index, p.smooth))
}

#[derive(Deserialize)]
struct SegmentParams {
    #[serde(flatten)]
    at: At,
    segment: usize,
    curve: bool,
}

fn set_segment(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: SegmentParams = parse(params)?;
    edit(s, p.at, |np, _| np.set_segment_curve(p.at.subpath, p.segment, p.curve))
}

#[derive(Deserialize)]
struct ClosedParams {
    #[serde(flatten)]
    at: At,
    closed: bool,
}

fn set_closed(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: ClosedParams = parse(params)?;
    edit(s, p.at, |np, _| np.set_closed(p.at.subpath, p.closed))
}

/// Runs `f` on the shape's nodes (with the world-to-local transform) and stores the result.
fn edit(s: &mut Session, at: At, f: impl FnOnce(&mut NodePath, Affine) -> Result<(), NodeError>) -> Result<Value, EngineError> {
    let shape = s.doc.shape_mut(at.id).ok_or_else(|| EngineError::NotFound(format!("shape {} does not exist", at.id)))?;
    let ShapeKind::Path { path } = &mut shape.kind else {
        return Err(EngineError::BadParams(format!("{} is not an editable path (text and images must be converted first)", shape.name)));
    };
    let mut np = NodePath::from_bez(path).map_err(node_error)?;
    f(&mut np, shape.transform.inverse()).map_err(node_error)?;
    *path = np.to_bez();
    Ok(json!({ "id": at.id, "nodes": np.node_count() }))
}

fn node_error(e: NodeError) -> EngineError {
    EngineError::BadParams(e.to_string())
}

#[derive(Deserialize)]
struct CreateParams {
    nodes: Vec<NodeSpec>,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    layer: LayerId,
}

#[derive(Deserialize)]
struct NodeSpec {
    x: f64,
    y: f64,
    #[serde(rename = "in")]
    in_tip: Option<[f64; 2]>,
    out: Option<[f64; 2]>,
    #[serde(default)]
    smooth: bool,
}

fn create(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: CreateParams = parse(params)?;
    let minimum = if p.closed { 3 } else { 2 };
    if p.nodes.len() < minimum || p.nodes.len() > MAX_NODES {
        return Err(EngineError::BadParams(format!("a path needs between {minimum} and {MAX_NODES} nodes")));
    }
    let mut nodes = Vec::with_capacity(p.nodes.len());
    for n in &p.nodes {
        let tips = [n.in_tip, n.out].into_iter().flatten().flatten();
        finite(&[n.x, n.y])?;
        finite(&tips.collect::<Vec<f64>>())?;
        let at = Point::new(n.x, n.y);
        let rel = |t: Option<[f64; 2]>| t.map(|t| Point::new(t[0], t[1]) - at);
        nodes.push(Node { p: at, in_h: rel(n.in_tip), out_h: rel(n.out), smooth: n.smooth });
    }
    let np = NodePath { subpaths: vec![laserfoundry_geom::nodes::SubPath { nodes, closed: p.closed }] };
    if s.doc.layer(p.layer).is_none() {
        return Err(EngineError::NotFound(format!("layer {} does not exist", p.layer)));
    }
    let id = s.doc.add_shape(Shape::path("Path", p.layer, np.to_bez()));
    s.selection = vec![id];
    Ok(json!({ "id": id }))
}

#[derive(Deserialize, Default)]
struct Ids {
    ids: Option<Vec<ShapeId>>,
}

fn convert_to_path(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: Ids = parse(params)?;
    let ids: Vec<ShapeId> = id_list(&p.ids, &s.selection).into_iter().filter(|id| s.doc.shape(*id).is_some()).collect();
    if ids.is_empty() {
        return Err(EngineError::NothingSelected);
    }
    let mut converted = 0;
    for id in ids {
        if let Some(shape) = s.doc.shape_mut(id)
            && let ShapeKind::Text { path, .. } = &shape.kind
        {
            let path: BezPath = path.clone();
            shape.kind = ShapeKind::Path { path };
            converted += 1;
        }
    }
    if converted == 0 {
        return Err(EngineError::BadParams("only text needs converting; paths are already editable".into()));
    }
    Ok(json!({ "converted": converted }))
}
