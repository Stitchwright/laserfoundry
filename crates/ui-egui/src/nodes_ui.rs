//! The Nodes and Pen tools: gestures that turn into `node.*` and `path.create` commands.
//!
//! Node geometry lives in `laserfoundry_geom::nodes` (shared with the engine), so a drag preview
//! here is computed with the very same code that will be applied when the mouse is released.

use egui::{Color32, PointerButton, Pos2, Shape, Stroke, vec2};
use laserfoundry_doc::{ShapeId, ShapeKind};
use laserfoundry_geom::nodes::{Node, NodePath, Side, SubPath};
use laserfoundry_geom::{Point, Vec2};
use serde_json::json;

use crate::LaserFoundryApp;
use crate::canvas::Snapper;
use crate::view::View;

/// Which node of which shape is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeSel {
    pub shape: ShapeId,
    pub subpath: usize,
    pub index: usize,
}

/// What is being dragged and where its preview currently is (workspace mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeDrag {
    pub shape: ShapeId,
    pub subpath: usize,
    pub index: usize,
    pub target: DragTarget,
    pub to: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragTarget {
    Node,
    Handle(Side),
}

/// A path being drawn with the pen.
#[derive(Debug, Clone, Default)]
pub struct Pen {
    pub nodes: Vec<Node>,
    /// The last node is being dragged out into handles.
    pub dragging: bool,
}

/// The selected path's nodes in workspace coordinates, when exactly one editable path is selected.
pub fn world_nodes(app: &LaserFoundryApp) -> Option<(ShapeId, NodePath)> {
    let [id] = app.session.selection.as_slice() else { return None };
    let shape = app.session.doc.shape(*id)?;
    let ShapeKind::Path { path } = &shape.kind else { return None };
    let mut world = path.clone();
    world.apply_affine(shape.transform);
    Some((*id, NodePath::from_bez(&world).ok()?))
}

// ---- Nodes tool ----------------------------------------------------------------------------

pub fn interact(app: &mut LaserFoundryApp, ui: &egui::Ui, resp: &egui::Response, view: &View, pointer: Option<Point>, snapper: &Snapper) {
    if app.tool == crate::Tool::Pen {
        return pen_interact(app, ui, resp, view, pointer, snapper);
    }
    let tol = f64::from(app.hit_px() / view.zoom) * 1.2;
    let pressed = ui.input(|i| i.pointer.press_origin()).map(|p| view.to_world(p));

    if resp.drag_started_by(PointerButton::Primary)
        && let Some(p0) = pressed
        && let Some((shape, np)) = world_nodes(app)
    {
        app.node_drag = hit(app, &np, p0, tol).map(|(subpath, index, target)| NodeDrag { shape, subpath, index, target, to: p0 });
        if let Some(d) = app.node_drag {
            app.node_sel = Some(NodeSel { shape, subpath: d.subpath, index: d.index });
        }
    }
    if let (Some(d), Some(p)) = (app.node_drag.as_mut(), pointer) {
        d.to = snapper.point(p).point;
    }
    if resp.drag_stopped_by(PointerButton::Primary)
        && let Some(d) = app.node_drag.take()
    {
        commit_drag(app, d);
    }
    if resp.double_clicked_by(PointerButton::Primary)
        && let Some(p) = pointer
    {
        insert_at(app, p, tol);
    } else if resp.clicked_by(PointerButton::Primary)
        && let Some(p) = pointer
    {
        click(app, p, tol);
    }
}

/// The node or handle under `p`: `(subpath, index, what)`. Handles belong to the selected node
/// only, and win over nodes (they can sit on top of a neighbour).
fn hit(app: &LaserFoundryApp, np: &NodePath, p: Point, tol: f64) -> Option<(usize, usize, DragTarget)> {
    if let Some(sel) = app.node_sel
        && let Some(node) = np.subpaths.get(sel.subpath).and_then(|s| s.nodes.get(sel.index))
    {
        for (side, h) in [(Side::In, node.in_h), (Side::Out, node.out_h)] {
            if let Some(h) = h
                && (node.p + h).distance(p) <= tol
            {
                return Some((sel.subpath, sel.index, DragTarget::Handle(side)));
            }
        }
    }
    np.subpaths.iter().enumerate().find_map(|(si, sp)| sp.nodes.iter().position(|n| n.p.distance(p) <= tol).map(|i| (si, i, DragTarget::Node)))
}

fn click(app: &mut LaserFoundryApp, p: Point, tol: f64) {
    if let Some((shape, np)) = world_nodes(app)
        && let Some((subpath, index, _)) = hit(app, &np, p, tol)
    {
        app.node_sel = Some(NodeSel { shape, subpath, index });
        return;
    }
    app.node_sel = None;
    // Clicking another shape switches which path is being edited.
    if let Some(id) = app.geometry.pick(p, tol) {
        app.run("edit.select", json!({ "ids": [id] }));
    }
}

fn insert_at(app: &mut LaserFoundryApp, p: Point, tol: f64) {
    let Some((shape, np)) = world_nodes(app) else { return };
    if hit(app, &np, p, tol).is_some() {
        return;
    }
    let Some(near) = np.nearest(p).filter(|n| n.distance <= tol * 2.0) else { return };
    if let Some(v) = app.run("node.insert", json!({ "id": shape, "subpath": near.subpath, "segment": near.segment, "t": near.t })) {
        let index = v["index"].as_u64().unwrap_or(0) as usize;
        app.node_sel = Some(NodeSel { shape, subpath: near.subpath, index });
    }
}

fn commit_drag(app: &mut LaserFoundryApp, d: NodeDrag) {
    let (id, sub, idx, x, y) = (d.shape, d.subpath, d.index, d.to.x, d.to.y);
    match d.target {
        DragTarget::Node => app.run("node.move", json!({ "id": id, "subpath": sub, "index": idx, "x": x, "y": y })),
        DragTarget::Handle(side) => {
            let side = if side == Side::In { "in" } else { "out" };
            app.run("node.move_handle", json!({ "id": id, "subpath": sub, "index": idx, "side": side, "x": x, "y": y }))
        }
    };
}

/// Deletes the selected node (the Delete key while the Nodes tool is active).
pub fn delete_selected(app: &mut LaserFoundryApp) {
    let Some(sel) = app.node_sel else { return };
    if app.run("node.delete", json!({ "id": sel.shape, "subpath": sel.subpath, "index": sel.index })).is_some() {
        app.node_sel = Some(NodeSel { index: sel.index.saturating_sub(1), ..sel });
    }
}

/// The path as it will look if the current drag is dropped now.
fn preview(np: &NodePath, d: &NodeDrag) -> NodePath {
    let mut p = np.clone();
    let _ = match d.target {
        DragTarget::Node => p.move_node(d.subpath, d.index, d.to),
        DragTarget::Handle(side) => p.move_handle(d.subpath, d.index, side, d.to),
    };
    p
}

// ---- Pen tool ------------------------------------------------------------------------------

/// Distance (screen points) within which clicking the first node closes the path.
const CLOSE_PX: f64 = 9.0;

fn pen_interact(app: &mut LaserFoundryApp, ui: &egui::Ui, resp: &egui::Response, view: &View, pointer: Option<Point>, snapper: &Snapper) {
    let close_tol = CLOSE_PX / f64::from(view.zoom);
    let pressed = ui.input(|i| i.pointer.press_origin()).map(|p| view.to_world(p));

    if resp.drag_started_by(PointerButton::Primary)
        && let Some(p0) = pressed
    {
        let p0 = snapper.point(p0).point;
        let pen = app.pen.get_or_insert_with(Pen::default);
        if closes(pen, p0, close_tol) {
            return finish(app, true);
        }
        pen.nodes.push(Node { p: p0, in_h: None, out_h: None, smooth: true });
        pen.dragging = true;
    }
    if let (Some(pen), Some(p)) = (app.pen.as_mut(), pointer)
        && pen.dragging
        && let Some(last) = pen.nodes.last_mut()
    {
        // Dragging out of a new node pulls its outgoing handle toward the cursor and mirrors the
        // incoming one, which is what makes a smooth curve.
        let tip = snapper.point(p).point;
        let h = tip - last.p;
        if h.hypot() * f64::from(view.zoom) > 3.0 {
            last.out_h = Some(h);
            last.in_h = Some(-h);
        }
    }
    if resp.drag_stopped_by(PointerButton::Primary)
        && let Some(pen) = app.pen.as_mut()
    {
        pen.dragging = false;
    }
    if resp.double_clicked_by(PointerButton::Primary) {
        // The first click of the double click already added the node at this spot, and this
        // second click returns here before it could add a duplicate.
        return finish(app, false);
    }
    if resp.clicked_by(PointerButton::Primary)
        && let Some(p) = pointer
    {
        let p = snapper.point(p).point;
        let pen = app.pen.get_or_insert_with(Pen::default);
        if closes(pen, p, close_tol) {
            return finish(app, true);
        }
        pen.nodes.push(Node::corner(p));
    }
    if resp.clicked_by(PointerButton::Secondary) {
        finish(app, false);
    }
}

fn closes(pen: &Pen, p: Point, tol: f64) -> bool {
    pen.nodes.len() >= 3 && pen.nodes.first().is_some_and(|n| n.p.distance(p) <= tol)
}

/// Turns the pen's nodes into a path (closed or open). Fewer than two nodes just cancels.
pub fn finish(app: &mut LaserFoundryApp, closed: bool) {
    let Some(pen) = app.pen.take() else { return };
    if pen.nodes.len() < 2 {
        return;
    }
    let nodes: Vec<serde_json::Value> = pen
        .nodes
        .iter()
        .map(|n| {
            let tip = |h: Option<Vec2>| h.map(|h| [n.p.x + h.x, n.p.y + h.y]);
            json!({ "x": n.p.x, "y": n.p.y, "in": tip(n.in_h), "out": tip(n.out_h), "smooth": n.smooth })
        })
        .collect();
    let layer = app.active_layer;
    app.run("path.create", json!({ "nodes": nodes, "closed": closed && pen.nodes.len() >= 3, "layer": layer }));
}

pub fn cancel(app: &mut LaserFoundryApp) {
    app.pen = None;
    app.node_drag = None;
}

// ---- drawing -------------------------------------------------------------------------------

pub fn draw(app: &LaserFoundryApp, painter: &egui::Painter, view: &View) {
    let accent = app.tokens.selection;
    match app.tool {
        crate::Tool::Nodes => draw_nodes(app, painter, view, accent),
        crate::Tool::Pen => draw_pen(app, painter, view, accent),
        _ => {}
    }
}

fn draw_nodes(app: &LaserFoundryApp, painter: &egui::Painter, view: &View, accent: Color32) {
    let Some((_, np)) = world_nodes(app) else { return };
    let shown = match &app.node_drag {
        Some(d) => {
            let p = preview(&np, d);
            let lines = laserfoundry_geom::flatten(&p.to_bez(), 0.05);
            for l in &lines {
                let pts: Vec<Pos2> = l.points.iter().map(|q| view.to_screen(*q)).collect();
                painter.add(if l.closed { Shape::closed_line(pts, Stroke::new(1.5, accent)) } else { Shape::line(pts, Stroke::new(1.5, accent)) });
            }
            p
        }
        None => np,
    };
    for (si, sp) in shown.subpaths.iter().enumerate() {
        for (i, node) in sp.nodes.iter().enumerate() {
            let selected = app.node_sel.is_some_and(|s| s.subpath == si && s.index == i);
            let c = view.to_screen(node.p);
            let r = egui::Rect::from_center_size(c, vec2(7.0, 7.0));
            let stroke = Stroke::new(1.0, accent);
            if selected {
                painter.rect_filled(r, 1.0, accent);
                for h in [node.in_h, node.out_h].into_iter().flatten() {
                    let tip = view.to_screen(node.p + h);
                    painter.line_segment([c, tip], Stroke::new(1.0, accent.gamma_multiply(0.6)));
                    painter.circle_filled(tip, 3.5, Color32::WHITE);
                    painter.circle_stroke(tip, 3.5, stroke);
                }
            } else {
                painter.rect_filled(r, 1.0, Color32::WHITE);
                painter.rect_stroke(r, 1.0, stroke, egui::StrokeKind::Outside);
            }
            if node.smooth {
                painter.circle_stroke(c, 6.0, Stroke::new(1.0, accent.gamma_multiply(0.5)));
            }
        }
    }
}

fn draw_pen(app: &LaserFoundryApp, painter: &egui::Painter, view: &View, accent: Color32) {
    let Some(pen) = app.pen.as_ref() else { return };
    // The path so far, plus a segment to the cursor.
    let mut nodes = pen.nodes.clone();
    if let (Some(h), false) = (app.hover_world, pen.dragging) {
        nodes.push(Node::corner(h));
    }
    if nodes.len() >= 2 {
        let np = NodePath { subpaths: vec![SubPath { nodes, closed: false }] };
        for l in laserfoundry_geom::flatten(&np.to_bez(), 0.05) {
            let pts: Vec<Pos2> = l.points.iter().map(|q| view.to_screen(*q)).collect();
            painter.add(Shape::line(pts, Stroke::new(1.5, accent)));
        }
    }
    for (i, n) in pen.nodes.iter().enumerate() {
        let c = view.to_screen(n.p);
        let first = i == 0 && pen.nodes.len() >= 3;
        painter.circle_filled(c, if first { 5.0 } else { 3.5 }, if first { Color32::WHITE } else { accent });
        for h in [n.in_h, n.out_h].into_iter().flatten() {
            let tip = view.to_screen(n.p + h);
            painter.line_segment([c, tip], Stroke::new(1.0, accent.gamma_multiply(0.6)));
            painter.circle_stroke(tip, 3.0, Stroke::new(1.0, accent));
        }
    }
}
