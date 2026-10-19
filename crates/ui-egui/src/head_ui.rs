//! Showing where the laser head is on the canvas, and clicking to send it somewhere.
//!
//! This only works when the design's coordinates are machine coordinates ("Absolute" start), the
//! one case where a machine position and a point on the bed are the same thing.

use crate::i18n::tr;
use laserfoundry_doc::{Document, Machine, Origin, StartFrom};
use laserfoundry_geom::Point;

use crate::LaserFoundryApp;
use crate::view::View;

/// Whether machine positions can be mapped onto the bed at all.
pub fn mappable(doc: &Document) -> bool {
    doc.job_origin.start_from == StartFrom::Absolute
}

/// The bed point for a machine position.
pub fn machine_to_world(m: &Machine, x: f64, y: f64) -> Point {
    match m.origin {
        Origin::TopLeft => Point::new(x, y),
        Origin::BottomLeft => Point::new(x, m.bed_height - y),
    }
}

/// The machine position for a bed point, or `None` when the point is off the bed or not finite.
pub fn world_to_machine(m: &Machine, p: Point) -> Option<(f64, f64)> {
    if !(p.x.is_finite() && p.y.is_finite()) || p.x < 0.0 || p.y < 0.0 || p.x > m.bed_width || p.y > m.bed_height {
        return None;
    }
    Some(match m.origin {
        Origin::TopLeft => (p.x, p.y),
        Origin::BottomLeft => (p.x, m.bed_height - p.y),
    })
}

/// Where the head is on the bed, when the link reports it and the mapping is meaningful.
pub fn head_world(app: &LaserFoundryApp) -> Option<Point> {
    if !mappable(&app.session.doc) {
        return None;
    }
    let status = app.snap.as_ref()?.status.as_ref()?;
    if status.position_is_work {
        return None; // work coordinates move with the offset; the bed position is unknown
    }
    let (x, y, _) = status.position?;
    Some(machine_to_world(&app.session.doc.machine, x, y))
}

pub fn draw(app: &LaserFoundryApp, painter: &egui::Painter, view: &View) {
    if let Some(p) = head_world(app) {
        crate::timeline_ui::draw_head(painter, view.to_screen(p), app.tokens.error);
    }
}

/// Handles a canvas click while "move the laser" is on. Returns whether the click was used.
pub fn click(app: &mut LaserFoundryApp, at: Point) -> bool {
    if !app.move_head_mode {
        return false;
    }
    if !mappable(&app.session.doc) {
        app.status = Some((tr("Moving the laser from the canvas needs Start from: Absolute coordinates"), true));
        return true;
    }
    let Some((x, y)) = world_to_machine(&app.session.doc.machine, at) else {
        app.status = Some((tr("That point is outside the bed"), true));
        return true;
    };
    let feed = app.jog_feed;
    app.link_do(|l| l.jog_to(x, y, feed));
    true
}
