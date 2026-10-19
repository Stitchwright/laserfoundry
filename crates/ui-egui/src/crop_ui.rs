//! Cropping a picture by dragging its edges on the canvas. The edges are those of the picture itself
//! (so rotated and flipped pictures work), and the result lands in the same percentages the
//! inspector shows; the crop is applied with the Crop button.

use egui::{Color32, Stroke};
use laserfoundry_doc::ShapeKind;
use laserfoundry_geom::{Affine, Point};

use crate::LaserFoundryApp;
use crate::view::View;

/// How close (screen points) the pointer must be to an edge to grab it.
const GRAB_PX: f32 = 10.0;
/// The most that can be cut from a side in percent, matching the inspector.
pub const MAX_PCT: f64 = 95.0;

/// Edge indices, in the order of `crop_pct`: left, top, right, bottom.
pub fn selected_image(app: &LaserFoundryApp) -> Option<(Affine, f64, f64)> {
    let [id] = app.session.selection.as_slice() else { return None };
    match &app.session.doc.shape(*id)?.kind {
        ShapeKind::Image { width_mm, height_mm, .. } => Some((app.session.doc.shape(*id)?.transform, *width_mm, *height_mm)),
        _ => None,
    }
}

/// The picture's corners on screen: top-left, top-right, bottom-right, bottom-left.
fn corners(view: &View, transform: Affine, w: f64, h: f64) -> [egui::Pos2; 4] {
    [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)].map(|p| view.to_screen(transform * p))
}

/// The edge nearest `pointer` within reach, if any.
pub fn edge_at(view: &View, transform: Affine, w: f64, h: f64, pointer: egui::Pos2) -> Option<usize> {
    let c = corners(view, transform, w, h);
    // left: c3-c0, top: c0-c1, right: c1-c2, bottom: c2-c3
    let edges = [(c[3], c[0]), (c[0], c[1]), (c[1], c[2]), (c[2], c[3])];
    edges
        .iter()
        .enumerate()
        .map(|(i, (a, b))| (i, distance_to_segment(pointer, *a, *b)))
        .filter(|(_, d)| *d <= GRAB_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

fn distance_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
}

/// The percentage of the picture to cut off `edge` when the pointer is at world point `at`.
pub fn pct_for(edge: usize, transform: Affine, w: f64, h: f64, at: Point) -> f64 {
    let local = transform.inverse() * at;
    let (x, y) = (local.x, local.y);
    let pct = match edge {
        0 => x / w,
        1 => y / h,
        2 => (w - x) / w,
        _ => (h - y) / h,
    } * 100.0;
    if pct.is_finite() { pct.clamp(0.0, MAX_PCT) } else { 0.0 }
}

/// Handles the pointer while crop mode is on. Returns whether the canvas should ignore it.
pub fn interact(app: &mut LaserFoundryApp, resp: &egui::Response, view: &View, pointer: Option<Point>) -> bool {
    let Some((transform, w, h)) = selected_image(app) else {
        app.crop_mode = false;
        return false;
    };
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(p) = resp.interact_pointer_pos()
    {
        app.crop_drag = edge_at(view, transform, w, h, p).or_else(|| nearest_cut_edge(app, view, transform, w, h, p));
    }
    if let (Some(edge), Some(p)) = (app.crop_drag, pointer)
        && let Some(slot) = app.crop_pct.get_mut(edge)
    {
        *slot = pct_for(edge, transform, w, h, p);
        // Opposite sides together must leave something.
        let opposite = (edge + 2) % 4;
        if let Some(other) = app.crop_pct.get(opposite).copied()
            && *app.crop_pct.get(edge).unwrap_or(&0.0) + other > MAX_PCT
            && let Some(s) = app.crop_pct.get_mut(edge)
        {
            *s = (MAX_PCT - other).max(0.0);
        }
    }
    if resp.drag_stopped_by(egui::PointerButton::Primary) {
        app.crop_drag = None;
    }
    true
}

/// A cut edge can be grabbed along its (inset) line, not just the picture's own border.
fn nearest_cut_edge(app: &LaserFoundryApp, view: &View, transform: Affine, w: f64, h: f64, p: egui::Pos2) -> Option<usize> {
    let [l, t, r, b] = app.crop_pct.map(|v| v / 100.0);
    let (x0, y0, x1, y1) = (l * w, t * h, w - r * w, h - b * h);
    let s = |x: f64, y: f64| view.to_screen(transform * Point::new(x, y));
    let edges = [(s(x0, y1), s(x0, y0)), (s(x0, y0), s(x1, y0)), (s(x1, y0), s(x1, y1)), (s(x1, y1), s(x0, y1))];
    edges
        .iter()
        .enumerate()
        .map(|(i, (a, b))| (i, distance_to_segment(p, *a, *b)))
        .filter(|(_, d)| *d <= GRAB_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

pub fn draw(app: &LaserFoundryApp, painter: &egui::Painter, view: &View) {
    if !app.crop_mode {
        return;
    }
    let Some((transform, w, h)) = selected_image(app) else { return };
    let [l, t, r, b] = app.crop_pct.map(|v| v / 100.0);
    let (x0, y0, x1, y1) = (l * w, t * h, w - r * w, h - b * h);
    let s = |x: f64, y: f64| view.to_screen(transform * Point::new(x, y));
    let shade = Color32::from_black_alpha(130);
    // The removed bands, as quads in the picture's own axes.
    for quad in [
        [s(0.0, 0.0), s(w, 0.0), s(w, y0), s(0.0, y0)],
        [s(0.0, y1), s(w, y1), s(w, h), s(0.0, h)],
        [s(0.0, y0), s(x0, y0), s(x0, y1), s(0.0, y1)],
        [s(x1, y0), s(w, y0), s(w, y1), s(x1, y1)],
    ] {
        painter.add(egui::Shape::convex_polygon(quad.to_vec(), shade, Stroke::NONE));
    }
    let accent = app.tokens.selection;
    let kept = [s(x0, y0), s(x1, y0), s(x1, y1), s(x0, y1)];
    painter.add(egui::Shape::closed_line(kept.to_vec(), Stroke::new(1.5, accent)));
    for (i, (a, b)) in [(kept[3], kept[0]), (kept[0], kept[1]), (kept[1], kept[2]), (kept[2], kept[3])].into_iter().enumerate() {
        let width = if app.crop_drag == Some(i) { 3.0 } else { 2.0 };
        painter.line_segment([a, b], Stroke::new(width, accent));
    }
}
