//! Snapping and ruler maths, kept free of egui so it can be tested exhaustively.
//!
//! Everything is in millimetres on the workspace (y down).

use laserfoundry_geom::{Point, Rect};

/// How close (in screen points) something must be to attract the cursor.
pub const SNAP_RADIUS_PX: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapSettings {
    /// Snap to this grid pitch (mm) when no object is closer; `None` turns grid snapping off.
    pub grid: Option<f64>,
    /// Snap to the edges, centres and mid-lines of other shapes, the bed, and guides.
    pub objects: bool,
    /// Which angled lines tracking offers through an acquired point, besides its row and column.
    pub angles: TrackAngles,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self { grid: None, objects: true, angles: TrackAngles::default() }
    }
}

/// The set of angled tracking lines (see [`TRACK_ANGLES`] for the full set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackAngles {
    /// Only the row and column through the point.
    RowColumn,
    /// 45 and 135 degrees.
    Diagonals,
    /// 30, 45 and 60 degrees and their mirror images.
    #[default]
    ThirtySixty,
    /// Every 15 degrees.
    Fifteen,
}

impl TrackAngles {
    pub const ALL: [TrackAngles; 4] = [Self::RowColumn, Self::Diagonals, Self::ThirtySixty, Self::Fifteen];

    /// Angles in degrees, measured from +x with y down; the row and column are not listed.
    pub fn degrees(self) -> &'static [f64] {
        match self {
            Self::RowColumn => &[],
            Self::Diagonals => &[45.0, 135.0],
            Self::ThirtySixty => &TRACK_ANGLES,
            Self::Fifteen => &[15.0, 30.0, 45.0, 60.0, 75.0, 105.0, 120.0, 135.0, 150.0, 165.0],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RowColumn => "Row and column only",
            Self::Diagonals => "45\u{b0} diagonals",
            Self::ThirtySixty => "30\u{b0}, 45\u{b0}, 60\u{b0}",
            Self::Fifteen => "Every 15\u{b0}",
        }
    }
}

/// What a snap target is, which decides its marker and its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// A side of the bed.
    Bed,
    /// The middle of the bed.
    BedCenter,
    /// A guide pulled out of a ruler.
    Guide,
    /// A side of a shape's bounding box.
    Edge,
    /// The middle of a shape's bounding box.
    Center,
    /// The middle of a straight segment.
    Midpoint,
    /// A corner or segment end of a drawn path.
    Endpoint,
    /// A line through a point the person acquired by hovering over it.
    Tracked,
}

impl TargetKind {
    /// Wins when several targets are equally close: the more specific one.
    fn rank(self) -> u8 {
        match self {
            Self::Tracked => 0,
            Self::Guide => 1,
            Self::Bed => 2,
            Self::Edge => 3,
            Self::Center | Self::BedCenter => 4,
            Self::Midpoint => 5,
            Self::Endpoint => 6,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bed => "Bed edge",
            Self::BedCenter => "Bed center",
            Self::Guide => "Guide",
            Self::Edge => "Edge",
            Self::Center => "Center",
            Self::Midpoint => "Midpoint",
            Self::Endpoint => "Endpoint",
            Self::Tracked => "Tracking",
        }
    }
}

/// One coordinate the cursor can lock onto, with what it belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// The coordinate on the snapping axis, in mm.
    pub at: f64,
    pub kind: TargetKind,
    /// How far the owner extends along the other axis (`None`: unbounded, like a guide). A point's span is one value.
    pub span: Option<(f64, f64)>,
}

/// Coordinates the cursor can lock onto, per axis.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapTargets {
    pub xs: Vec<Target>,
    pub ys: Vec<Target>,
}

impl SnapTargets {
    /// Adds a shape's edges and centre lines.
    pub fn add_rect(&mut self, r: Rect) {
        self.add_rect_as(r, TargetKind::Edge, TargetKind::Center);
    }

    /// Adds the bed's sides and centre lines.
    pub fn add_bed(&mut self, r: Rect) {
        self.add_rect_as(r, TargetKind::Bed, TargetKind::BedCenter);
    }

    fn add_rect_as(&mut self, r: Rect, edge: TargetKind, center: TargetKind) {
        let (xspan, yspan) = (Some((r.y0, r.y1)), Some((r.x0, r.x1)));
        for (at, kind) in [(r.x0, edge), ((r.x0 + r.x1) / 2.0, center), (r.x1, edge)] {
            self.xs.push(Target { at, kind, span: xspan });
        }
        for (at, kind) in [(r.y0, edge), ((r.y0 + r.y1) / 2.0, center), (r.y1, edge)] {
            self.ys.push(Target { at, kind, span: yspan });
        }
    }

    /// Adds the vertical guide at `x`, or the horizontal one at `y`.
    pub fn add_guide(&mut self, vertical: bool, pos: f64) {
        let target = Target { at: pos, kind: TargetKind::Guide, span: None };
        if vertical { self.xs.push(target) } else { self.ys.push(target) }
    }

    /// Adds the lines through a point (a vertex or midpoint).
    pub fn add_point(&mut self, p: Point, kind: TargetKind) {
        self.xs.push(Target { at: p.x, kind, span: Some((p.y, p.y)) });
        self.ys.push(Target { at: p.y, kind, span: Some((p.x, p.x)) });
    }
}

/// Most vertex targets taken from one shape, and from the whole drawing.
pub const MAX_VERTEX_TARGETS_PER_SHAPE: usize = 200;
pub const MAX_VERTEX_TARGETS: usize = 5000;

/// The points of a path worth snapping to, in workspace mm: every segment end, plus the midpoint
/// of each straight segment. Stops after `cap` points.
pub fn path_points(path: &kurbo::BezPath, transform: kurbo::Affine, cap: usize) -> Vec<(Point, TargetKind)> {
    use kurbo::PathEl;
    let mut out = Vec::new();
    let mut at: Option<Point> = None;
    let mut start: Option<Point> = None;
    for el in path.elements() {
        if out.len() >= cap {
            break;
        }
        match *el {
            PathEl::MoveTo(p) => {
                out.push((transform * p, TargetKind::Endpoint));
                at = Some(p);
                start = Some(p);
            }
            PathEl::LineTo(p) => {
                if let Some(a) = at {
                    out.push((transform * a.midpoint(p), TargetKind::Midpoint));
                }
                out.push((transform * p, TargetKind::Endpoint));
                at = Some(p);
            }
            PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                out.push((transform * p, TargetKind::Endpoint));
                at = Some(p);
            }
            PathEl::ClosePath => {
                if let (Some(a), Some(s)) = (at, start) {
                    out.push((transform * a.midpoint(s), TargetKind::Midpoint));
                    at = Some(s);
                }
            }
        }
    }
    out.truncate(cap);
    out.retain(|(p, _)| p.x.is_finite() && p.y.is_finite());
    out
}

/// The result of snapping one point: where it ended up and which target lines it locked onto
/// (for drawing the alignment guides).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapped {
    pub point: Point,
    pub x_line: Option<Target>,
    pub y_line: Option<Target>,
    /// An angled tracking line the point was pulled onto.
    pub diagonal: Option<Diagonal>,
}

/// Angles (degrees, y down) of the tracking lines through an acquired point, besides the row and
/// column: 30, 45 and 60 degrees and their mirror images.
pub const TRACK_ANGLES: [f64; 6] = [30.0, 45.0, 60.0, 120.0, 135.0, 150.0];

/// A line through an acquired point at `degrees` (measured from the +x axis, y pointing down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Diagonal {
    pub origin: Point,
    pub degrees: f64,
}

impl Diagonal {
    /// Unit direction along the line.
    pub fn direction(&self) -> (f64, f64) {
        let r = self.degrees.to_radians();
        (r.cos(), r.sin())
    }
}

/// Snaps a single point. `threshold` is in mm.
pub fn snap_point(p: Point, settings: &SnapSettings, targets: &SnapTargets, threshold: f64) -> Snapped {
    let (x, x_line) = snap_axis(p.x, settings, &targets.xs, threshold);
    let (y, y_line) = snap_axis(p.y, settings, &targets.ys, threshold);
    Snapped { point: Point::new(x, y), x_line, y_line, diagonal: None }
}

/// Snaps a translation of a group of coordinates (the moving selection's edges and centre): the
/// closest key-to-target alignment within `threshold` wins on each axis, else the grid snaps the
/// first key. Returns the adjusted `(dx, dy)` and the target lines locked onto.
pub fn snap_translation(
    delta: (f64, f64),
    keys: &SnapTargets,
    settings: &SnapSettings,
    targets: &SnapTargets,
    threshold: f64,
) -> ((f64, f64), Option<Target>, Option<Target>) {
    let (dx, x_line) = snap_delta_axis(delta.0, &keys.xs, settings, &targets.xs, threshold);
    let (dy, y_line) = snap_delta_axis(delta.1, &keys.ys, settings, &targets.ys, threshold);
    ((dx, dy), x_line, y_line)
}

/// Most points that can be acquired for tracking at once.
pub const MAX_TRACKED: usize = 6;

/// The nearest point the cursor is on (within `threshold` mm), looked for among the targets themselves:
/// vertices, midpoints and centres, and the corners, side middles and centre of each shape's box. Unlike
/// reading it back from a snapped result, this is not confused by points that share a row or column.
pub fn acquirable_near(p: Point, targets: &SnapTargets, threshold: f64) -> Option<Point> {
    if !(p.x.is_finite() && p.y.is_finite() && threshold.is_finite() && threshold >= 0.0) {
        return None;
    }
    let mut best: Option<(f64, Point)> = None;
    let mut consider = |c: Point| {
        let d = c.distance(p);
        if d.is_finite() && d <= threshold && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    };
    // Candidates come from the vertical lines (a point's column, a box's left, middle and right) and
    // the horizontal lines of box tops and bottoms; the box's x extent is read from the horizontal lines.
    for t in &targets.xs {
        let Some((a, b)) = t.span.filter(|(a, b)| a.is_finite() && b.is_finite()) else { continue };
        if (t.at - p.x).abs() > threshold || !t.at.is_finite() {
            continue;
        }
        match t.kind {
            TargetKind::Endpoint | TargetKind::Midpoint | TargetKind::Center | TargetKind::BedCenter => consider(Point::new(t.at, (a + b) / 2.0)),
            TargetKind::Edge => [a, (a + b) / 2.0, b].into_iter().for_each(|y| consider(Point::new(t.at, y))),
            _ => {}
        }
    }
    for t in &targets.ys {
        let Some((a, b)) = t.span.filter(|(a, b)| a.is_finite() && b.is_finite()) else { continue };
        if (t.at - p.y).abs() > threshold || !t.at.is_finite() {
            continue;
        }
        // Tops and bottoms of boxes (their corners are already found from the sides).
        if t.kind == TargetKind::Edge {
            consider(Point::new((a + b) / 2.0, t.at));
        }
    }
    best.map(|(_, c)| c)
}

/// [`snap_point`], then on each axis that no object caught, a horizontal or vertical line through
/// an acquired point within `threshold` pulls the result onto it (over the grid, too).
pub fn snap_point_tracked(p: Point, settings: &SnapSettings, targets: &SnapTargets, tracked: &[Point], threshold: f64) -> Snapped {
    let mut s = snap_point(p, settings, targets, threshold);
    let pull = |v: f64, along: fn(&Point) -> f64, other: fn(&Point) -> f64| {
        tracked
            .iter()
            .filter(|t| along(t).is_finite() && other(t).is_finite() && (along(t) - v).abs() <= threshold)
            .min_by(|a, b| (along(a) - v).abs().total_cmp(&(along(b) - v).abs()))
            .map(|t| (along(t), Target { at: along(t), kind: TargetKind::Tracked, span: Some((other(t), other(t))) }))
    };
    if settings.objects && p.x.is_finite() && p.y.is_finite() {
        if s.x_line.is_none()
            && let Some((x, target)) = pull(p.x, |t| t.x, |t| t.y)
        {
            s.point.x = x;
            s.x_line = Some(target);
        }
        if s.y_line.is_none()
            && let Some((y, target)) = pull(p.y, |t| t.y, |t| t.x)
        {
            s.point.y = y;
            s.y_line = Some(target);
        }
        pull_diagonal(&mut s, p, tracked, settings.angles.degrees(), threshold);
    }
    s
}

/// Pulls the point onto the nearest angled line through an acquired point: free in both axes
/// where nothing else caught, else along the one line that an axis lock already fixed (so it lands
/// on the crossing of a tracking or object line with the angled line).
fn pull_diagonal(s: &mut Snapped, raw: Point, tracked: &[Point], angles: &[f64], threshold: f64) {
    let mut best: Option<(f64, Point, Diagonal)> = None;
    for t in tracked.iter().filter(|t| t.x.is_finite() && t.y.is_finite()) {
        for &degrees in angles {
            let diagonal = Diagonal { origin: *t, degrees };
            let (c, sn) = diagonal.direction();
            let q = match (s.x_line.is_some(), s.y_line.is_some()) {
                (false, false) => {
                    // Foot of the perpendicular from the cursor onto the line.
                    let along = (raw.x - t.x) * c + (raw.y - t.y) * sn;
                    Point::new(t.x + along * c, t.y + along * sn)
                }
                (true, false) if c.abs() > 1e-9 => Point::new(s.point.x, t.y + sn / c * (s.point.x - t.x)),
                (false, true) if sn.abs() > 1e-9 => Point::new(t.x + c / sn * (s.point.y - t.y), s.point.y),
                _ => continue,
            };
            let d = q.distance(raw);
            // Right on the acquired point itself every angle fits, which is no guidance.
            if d.is_finite() && d <= threshold && q.distance(*t) > threshold && best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, q, diagonal));
            }
        }
    }
    if let Some((_, q, diagonal)) = best {
        s.point = q;
        s.diagonal = Some(diagonal);
    }
}

/// What tracking does to a moving selection: `keys` are the points of its box (corners, edge
/// midpoints, centre) at the current offset; the key that needs the smallest nudge to sit on a
/// tracking line decides. Axes in `locked` (already caught by an object) are left alone, and angled
/// lines are only used when neither is locked. Returns the nudge and what to draw.
pub fn track_translation(keys: &[Point], tracked: &[Point], angles: TrackAngles, locked: (bool, bool), threshold: f64) -> Option<TrackNudge> {
    if tracked.is_empty() || (locked.0 && locked.1) {
        return None;
    }
    let settings = SnapSettings { grid: None, objects: true, angles };
    let none = SnapTargets::default();
    let mut best: Option<(f64, TrackNudge)> = None;
    for k in keys {
        let s = snap_point_tracked(*k, &settings, &none, tracked, threshold);
        let take_x = !locked.0 && s.x_line.is_some();
        let take_y = !locked.1 && s.y_line.is_some();
        let take_diag = !locked.0 && !locked.1 && s.diagonal.is_some();
        if !(take_x || take_y || take_diag) {
            continue;
        }
        let nudge = ((if take_x || take_diag { s.point.x - k.x } else { 0.0 }), (if take_y || take_diag { s.point.y - k.y } else { 0.0 }));
        let cost = nudge.0.abs() + nudge.1.abs();
        if best.as_ref().is_none_or(|(c, _)| cost < *c) {
            best = Some((
                cost,
                TrackNudge {
                    by: nudge,
                    x: s.x_line.filter(|_| take_x || take_diag),
                    y: s.y_line.filter(|_| take_y || take_diag),
                    diagonal: s.diagonal.filter(|_| take_diag),
                },
            ));
        }
    }
    best.map(|(_, n)| n)
}

/// A nudge to a moving selection from tracking, and the lines to show for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackNudge {
    pub by: (f64, f64),
    pub x: Option<Target>,
    pub y: Option<Target>,
    pub diagonal: Option<Diagonal>,
}

fn snap_axis(v: f64, settings: &SnapSettings, targets: &[Target], threshold: f64) -> (f64, Option<Target>) {
    if !v.is_finite() {
        return (v, None);
    }
    if settings.objects
        && let Some(t) = nearest(v, targets, threshold)
    {
        return (t.at, Some(t));
    }
    match settings.grid {
        Some(step) if step.is_finite() && step > 0.0 => ((v / step).round() * step, None),
        _ => (v, None),
    }
}

fn snap_delta_axis(delta: f64, keys: &[Target], settings: &SnapSettings, targets: &[Target], threshold: f64) -> (f64, Option<Target>) {
    if !delta.is_finite() {
        return (delta, None);
    }
    if settings.objects {
        let mut best: Option<(f64, Target)> = None; // (adjustment, target line)
        for k in keys {
            if let Some(t) = nearest(k.at + delta, targets, threshold) {
                let adjust = t.at - (k.at + delta);
                if best
                    .is_none_or(|(a, bt)| adjust.abs() < a.abs() - 1e-9 || (adjust.abs() - a.abs()).abs() <= 1e-9 && t.kind.rank() > bt.kind.rank())
                {
                    best = Some((adjust, t));
                }
            }
        }
        if let Some((adjust, line)) = best {
            return (delta + adjust, Some(line));
        }
    }
    match (settings.grid, keys.first()) {
        (Some(step), Some(k0)) if step.is_finite() && step > 0.0 => (((k0.at + delta) / step).round() * step - k0.at, None),
        _ => (delta, None),
    }
}

/// The closest target within `threshold`; of equally close ones, the most specific.
fn nearest(v: f64, targets: &[Target], threshold: f64) -> Option<Target> {
    targets.iter().copied().filter(|t| t.at.is_finite() && (t.at - v).abs() <= threshold).min_by(|a, b| {
        let (da, db) = ((a.at - v).abs(), (b.at - v).abs());
        if (da - db).abs() <= 1e-9 { b.kind.rank().cmp(&a.kind.rank()) } else { da.total_cmp(&db) }
    })
}

/// The name to show beside a snap: the most specific thing the cursor or selection locked onto.
pub fn label(x: Option<&Target>, y: Option<&Target>) -> Option<&'static str> {
    [x, y].into_iter().flatten().max_by_key(|t| t.kind.rank()).map(|t| t.kind.label())
}

/// The shape of a mark drawn where things line up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// An X on an edge or corner.
    Cross,
    /// A + at a centre.
    Plus,
    /// A square at an endpoint.
    Square,
    /// A triangle at a midpoint.
    Triangle,
}

/// What to draw for one snapped axis, in workspace mm.
#[derive(Debug, Clone, PartialEq)]
pub struct Alignment {
    /// The alignment line's two ends.
    pub line: (Point, Point),
    pub marks: Vec<(Mark, Point)>,
    /// Tracking lines are dashed.
    pub dashed: bool,
}

/// Builds the alignment line and marks for a vertical line locked onto `hit` (pass `vertical:
/// false` for a horizontal one). `mover` is the moving selection's box, or a zero-size box at the
/// cursor; `extent` bounds lines whose owner has no span (guides). Both ends are ordered low to high.
pub fn alignment(vertical: bool, hit: &Target, mover: Rect, extent: Rect) -> Alignment {
    // Work in "line at u, running along v", then swap back for horizontal lines.
    let (mu, mv, ev) = if vertical {
        ((mover.x0, mover.x1), (mover.y0, mover.y1), (extent.y0, extent.y1))
    } else {
        ((mover.y0, mover.y1), (mover.x0, mover.x1), (extent.x0, extent.x1))
    };
    let u = hit.at;
    let span = hit.span.unwrap_or(ev);
    let tracked = hit.kind == TargetKind::Tracked;
    // A tracking line runs the whole view, like the construction lines of a CAD program.
    let (lo, hi) = if tracked { (ev.0.min(ev.1), ev.0.max(ev.1)) } else { (span.0.min(mv.0), span.1.max(mv.1)) };
    let at = |u: f64, v: f64| if vertical { Point::new(u, v) } else { Point::new(v, u) };
    let mut marks = Vec::new();
    let mid = |a: f64, b: f64| (a + b) / 2.0;
    match hit.kind {
        TargetKind::Center | TargetKind::BedCenter => marks.push((Mark::Plus, at(u, mid(span.0, span.1)))),
        TargetKind::Endpoint => marks.push((Mark::Square, at(u, span.0))),
        TargetKind::Midpoint => marks.push((Mark::Triangle, at(u, span.0))),
        TargetKind::Edge | TargetKind::Bed => {
            marks.push((Mark::Cross, at(u, span.0)));
            marks.push((Mark::Cross, at(u, span.1)));
        }
        TargetKind::Tracked => marks.push((Mark::Plus, at(u, span.0))),
        TargetKind::Guide => {}
    }
    if mv.1 - mv.0 > 1e-9 {
        // A moving box: mark where its edge, or centre, meets the line.
        if (mid(mu.0, mu.1) - u).abs() <= 1e-6 {
            marks.push((Mark::Plus, at(u, mid(mv.0, mv.1))));
        } else {
            marks.push((Mark::Cross, at(u, mv.0)));
            marks.push((Mark::Cross, at(u, mv.1)));
        }
    }
    Alignment { line: (at(u, lo), at(u, hi)), marks, dashed: tracked }
}

/// Shift while drawing a box or ellipse: the shorter side grows to match the longer, keeping the
/// direction the cursor moved in.
pub fn constrain_square(from: Point, to: Point) -> Point {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let side = dx.abs().max(dy.abs());
    Point::new(from.x + side.copysign(if dx == 0.0 { 1.0 } else { dx }), from.y + side.copysign(if dy == 0.0 { 1.0 } else { dy }))
}

/// Shift while drawing a line: the end point is rotated about the start to the nearest multiple
/// of `step_deg`, keeping its length.
pub fn constrain_angle(from: Point, to: Point, step_deg: f64) -> Point {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let len = dx.hypot(dy);
    if len == 0.0 || !(step_deg.is_finite() && step_deg > 0.0) {
        return to;
    }
    let step = step_deg.to_radians();
    let a = (dy.atan2(dx) / step).round() * step;
    Point::new(from.x + len * a.cos(), from.y + len * a.sin())
}

/// Ruler tick spacing in mm for a zoom of `px_per_mm`: `(major, minor)`. Majors are at least
/// ~60 px apart and always a 1-2-5 step; minors divide a major by five or ten.
pub fn ruler_steps(px_per_mm: f64) -> (f64, f64) {
    if !(px_per_mm.is_finite() && px_per_mm > 0.0) {
        return (100.0, 10.0);
    }
    let want = 60.0 / px_per_mm;
    let mut magnitude = 10f64.powf(want.log10().floor());
    // 1-2-5 sequence at this magnitude, then the next decade.
    let mut major = loop {
        let Some(m) = [1.0, 2.0, 5.0, 10.0].iter().map(|f| f * magnitude).find(|m| *m >= want) else {
            magnitude *= 10.0;
            continue;
        };
        break m;
    };
    major = major.clamp(1e-3, 1e6);
    let minor = if (major.log10() - major.log10().round()).abs() < 1e-9
        || (major / 5.0 - (major / 5.0).round()).abs() < 1e-9 && major.log10().fract().abs() > 0.1
    {
        major / 10.0
    } else {
        major / 5.0
    };
    (major, if minor * px_per_mm >= 5.0 { minor } else { major })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets() -> SnapTargets {
        let mut t = SnapTargets::default();
        t.add_rect(Rect::new(100.0, 50.0, 140.0, 90.0));
        t
    }

    mod alignment_tests {
        use super::*;

        const EXTENT: Rect = Rect { x0: 0.0, y0: 0.0, x1: 400.0, y1: 300.0 };

        fn cursor(x: f64, y: f64) -> Rect {
            Rect::new(x, y, x, y)
        }

        #[test]
        fn rectangles_become_edges_and_a_centre_that_remember_their_extent() {
            let mut t = SnapTargets::default();
            t.add_rect(Rect::new(10.0, 20.0, 50.0, 40.0));
            let kinds: Vec<_> = t.xs.iter().map(|x| (x.at, x.kind)).collect();
            assert_eq!(kinds, [(10.0, TargetKind::Edge), (30.0, TargetKind::Center), (50.0, TargetKind::Edge)]);
            assert!(t.xs.iter().all(|x| x.span == Some((20.0, 40.0))), "x lines run along y");
            assert!(t.ys.iter().all(|y| y.span == Some((10.0, 50.0))), "y lines run along x");
            let mut bed = SnapTargets::default();
            bed.add_bed(Rect::new(0.0, 0.0, 400.0, 300.0));
            assert_eq!(bed.xs.iter().map(|x| x.kind).collect::<Vec<_>>(), [TargetKind::Bed, TargetKind::BedCenter, TargetKind::Bed]);
            bed.add_guide(true, 77.0);
            assert_eq!(bed.xs.last().map(|g| (g.kind, g.span)), Some((TargetKind::Guide, None)));
        }

        #[test]
        fn of_equally_close_targets_the_more_specific_one_wins() {
            let mut t = SnapTargets::default();
            t.add_rect(Rect::new(0.0, 0.0, 100.0, 100.0)); // edge at x = 100
            t.add_point(Point::new(100.0, 40.0), TargetKind::Endpoint);
            t.add_point(Point::new(100.0, 60.0), TargetKind::Midpoint);
            let s = snap_point(Point::new(99.5, 40.0), &SnapSettings::default(), &t, 2.0);
            assert_eq!(s.x_line.map(|x| x.kind), Some(TargetKind::Endpoint));
            // A nearer edge still beats a farther endpoint.
            let mut t = SnapTargets::default();
            t.add_rect(Rect::new(0.0, 0.0, 100.0, 100.0));
            t.add_point(Point::new(101.5, 40.0), TargetKind::Endpoint);
            let s = snap_point(Point::new(99.8, 40.0), &SnapSettings::default(), &t, 2.0);
            assert_eq!(s.x_line.map(|x| x.kind), Some(TargetKind::Edge));
        }

        #[test]
        fn the_label_names_the_most_specific_hit() {
            let hit = |kind| Some(Target { at: 0.0, kind, span: None });
            assert_eq!(label(hit(TargetKind::Edge).as_ref(), hit(TargetKind::Endpoint).as_ref()), Some("Endpoint"));
            assert_eq!(label(None, hit(TargetKind::BedCenter).as_ref()), Some("Bed center"));
            assert_eq!(label(hit(TargetKind::Guide).as_ref(), hit(TargetKind::Bed).as_ref()), Some("Bed edge"));
            assert_eq!(label(None, None), None);
        }

        #[test]
        fn a_cursor_on_an_edge_gets_a_line_back_to_that_shape_with_crosses_on_its_corners() {
            let hit = Target { at: 50.0, kind: TargetKind::Edge, span: Some((10.0, 30.0)) };
            let a = alignment(true, &hit, cursor(50.0, 60.0), EXTENT);
            assert_eq!(a.line, (Point::new(50.0, 10.0), Point::new(50.0, 60.0)));
            assert_eq!(a.marks, [(Mark::Cross, Point::new(50.0, 10.0)), (Mark::Cross, Point::new(50.0, 30.0))]);
        }

        #[test]
        fn centres_get_a_plus_and_vertices_a_square_or_triangle() {
            let c = Target { at: 50.0, kind: TargetKind::Center, span: Some((10.0, 30.0)) };
            assert_eq!(alignment(true, &c, cursor(50.0, 5.0), EXTENT).marks, [(Mark::Plus, Point::new(50.0, 20.0))]);
            let v = Target { at: 50.0, kind: TargetKind::Endpoint, span: Some((12.0, 12.0)) };
            assert_eq!(alignment(true, &v, cursor(50.0, 12.0), EXTENT).marks, [(Mark::Square, Point::new(50.0, 12.0))]);
            let m = Target { kind: TargetKind::Midpoint, ..v };
            assert_eq!(alignment(true, &m, cursor(50.0, 12.0), EXTENT).marks, [(Mark::Triangle, Point::new(50.0, 12.0))]);
        }

        #[test]
        fn a_moving_box_is_marked_where_its_edge_or_centre_meets_the_line() {
            let hit = Target { at: 50.0, kind: TargetKind::Edge, span: Some((0.0, 10.0)) };
            let edge = alignment(true, &hit, Rect::new(50.0, 40.0, 70.0, 80.0), EXTENT);
            assert_eq!(edge.line, (Point::new(50.0, 0.0), Point::new(50.0, 80.0)));
            assert!(edge.marks.contains(&(Mark::Cross, Point::new(50.0, 40.0))) && edge.marks.contains(&(Mark::Cross, Point::new(50.0, 80.0))));
            let centre = alignment(true, &hit, Rect::new(40.0, 40.0, 60.0, 80.0), EXTENT);
            assert!(centre.marks.contains(&(Mark::Plus, Point::new(50.0, 60.0))), "{:?}", centre.marks);
        }

        #[test]
        fn horizontal_lines_mirror_the_vertical_ones() {
            let hit = Target { at: 20.0, kind: TargetKind::Endpoint, span: Some((5.0, 5.0)) };
            let a = alignment(false, &hit, cursor(30.0, 20.0), EXTENT);
            assert_eq!(a.line, (Point::new(5.0, 20.0), Point::new(30.0, 20.0)));
            assert_eq!(a.marks, [(Mark::Square, Point::new(5.0, 20.0))]);
        }

        #[test]
        fn guides_and_the_bed_centre_run_the_length_they_are_given() {
            let guide = Target { at: 70.0, kind: TargetKind::Guide, span: None };
            assert_eq!(alignment(true, &guide, cursor(70.0, 100.0), EXTENT).line, (Point::new(70.0, 0.0), Point::new(70.0, 300.0)));
            let centre = Target { at: 200.0, kind: TargetKind::BedCenter, span: Some((0.0, 300.0)) };
            let a = alignment(true, &centre, cursor(200.0, 10.0), EXTENT);
            assert_eq!(a.marks, [(Mark::Plus, Point::new(200.0, 150.0))]);
        }

        #[test]
        fn non_finite_numbers_do_not_produce_panics() {
            let hit = Target { at: f64::NAN, kind: TargetKind::Edge, span: Some((f64::INFINITY, f64::NAN)) };
            let _ = alignment(true, &hit, Rect::new(f64::NAN, 0.0, 1.0, f64::INFINITY), EXTENT);
            let _ = alignment(false, &hit, cursor(f64::NAN, f64::NAN), EXTENT);
        }
    }

    mod tracking_tests {
        use super::*;

        fn targets() -> SnapTargets {
            let mut t = SnapTargets::default();
            t.add_point(Point::new(40.0, 30.0), TargetKind::Endpoint);
            t
        }

        #[test]
        fn hovering_exactly_on_a_vertex_can_be_acquired() {
            assert_eq!(acquirable_near(Point::new(40.5, 29.5), &targets(), 2.0), Some(Point::new(40.0, 30.0)));
        }

        #[test]
        fn lining_up_with_two_different_things_is_not_a_point() {
            let mut t = targets();
            t.add_point(Point::new(90.0, 80.0), TargetKind::Endpoint);
            // x from the first vertex, y from the second: nothing is actually there.
            assert_eq!(acquirable_near(Point::new(40.0, 80.0), &t, 2.0), None);
            // A shape's own corner is a point (its two edge lines cross there); a point on one edge line is not.
            let mut e = SnapTargets::default();
            e.add_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
            assert_eq!(acquirable_near(Point::new(0.5, 0.5), &e, 2.0), Some(Point::new(0.0, 0.0)));
            assert_eq!(acquirable_near(Point::new(0.5, 7.0), &e, 0.8), None);
            // A shape's centre is a point: both centre lines come from the same rectangle.
            assert_eq!(acquirable_near(Point::new(5.2, 4.9), &e, 2.0), Some(Point::new(5.0, 5.0)));
        }

        #[test]
        fn a_tracked_point_pulls_the_cursor_onto_its_row_and_column() {
            let tracked = [Point::new(40.0, 30.0)];
            let none = SnapTargets::default();
            let s = snap_point_tracked(Point::new(100.0, 31.2), &SnapSettings::default(), &none, &tracked, 2.0);
            assert_eq!(s.point, Point::new(100.0, 30.0));
            assert_eq!(s.y_line.map(|t| (t.kind, t.at)), Some((TargetKind::Tracked, 30.0)));
            assert_eq!(s.x_line, None);
            let s = snap_point_tracked(Point::new(41.0, 90.0), &SnapSettings::default(), &none, &tracked, 2.0);
            assert_eq!(s.point, Point::new(40.0, 90.0));
            // Out of range, nothing happens.
            let s = snap_point_tracked(Point::new(100.0, 36.0), &SnapSettings::default(), &none, &tracked, 2.0);
            assert_eq!(s.point, Point::new(100.0, 36.0));
        }

        #[test]
        fn two_tracked_points_give_an_intersection_and_objects_still_win() {
            let tracked = [Point::new(40.0, 30.0), Point::new(90.0, 80.0)];
            let none = SnapTargets::default();
            let s = snap_point_tracked(Point::new(90.4, 29.6), &SnapSettings::default(), &none, &tracked, 2.0);
            assert_eq!(s.point, Point::new(90.0, 30.0), "column of one, row of the other");
            let mut t = SnapTargets::default();
            t.add_rect(Rect::new(0.0, 0.0, 100.0, 31.0));
            let s = snap_point_tracked(Point::new(50.0, 30.4), &SnapSettings::default(), &t, &tracked, 2.0);
            assert_eq!(s.y_line.map(|t| t.kind), Some(TargetKind::Edge), "an object on the axis keeps priority");
        }

        #[test]
        fn the_corners_middles_and_centre_of_one_shapes_box_can_be_acquired_but_not_lines_of_two() {
            let mut targets = SnapTargets::default();
            targets.add_rect(Rect::new(10.0, 20.0, 50.0, 60.0));
            targets.add_rect(Rect::new(100.0, 120.0, 140.0, 160.0));
            let at = |x: f64, y: f64| acquirable_near(Point::new(x, y), &targets, 1.0);
            assert_eq!(at(10.5, 20.4), Some(Point::new(10.0, 20.0)), "a corner");
            assert_eq!(at(50.0, 60.0), Some(Point::new(50.0, 60.0)), "the opposite corner");
            assert_eq!(at(30.0, 20.2), Some(Point::new(30.0, 20.0)), "the middle of the top side");
            assert_eq!(at(30.0, 40.0), Some(Point::new(30.0, 40.0)), "the centre");
            assert_eq!(at(120.0, 160.0), Some(Point::new(120.0, 160.0)), "the second shape's bottom middle");
            // The first shape's left line and the second shape's top line cross in empty space.
            assert_eq!(at(10.0, 120.0), None);
            assert_eq!(at(25.0, 33.0), None, "nothing locked");
            // Guides and the bed are not shapes' boxes.
            let mut guided = SnapTargets::default();
            guided.add_guide(true, 70.0);
            guided.add_guide(false, 80.0);
            assert_eq!(acquirable_near(Point::new(70.0, 80.0), &guided, 1.0), None);
            // Corners that share a row (both top corners of a box) are each found, not confused with one another.
            let mut shared = SnapTargets::default();
            for p in [(50.0, 50.0), (80.0, 50.0), (80.0, 80.0), (50.0, 80.0)] {
                shared.add_point(Point::new(p.0, p.1), TargetKind::Endpoint);
            }
            assert_eq!(acquirable_near(Point::new(80.0, 50.0), &shared, 1.0), Some(Point::new(80.0, 50.0)));
            assert_eq!(acquirable_near(Point::new(50.2, 80.3), &shared, 1.0), Some(Point::new(50.0, 80.0)));
            assert_eq!(acquirable_near(Point::new(65.0, 50.0), &shared, 1.0), None, "between the corners is not a point");
            assert_eq!(acquirable_near(Point::new(f64::NAN, 1.0), &shared, 1.0), None);
            assert_eq!(acquirable_near(Point::new(50.0, 50.0), &shared, f64::NAN), None);
        }

        #[test]
        fn tracking_overrides_the_grid_and_is_off_with_object_snapping() {
            let tracked = [Point::new(43.0, 33.0)];
            let none = SnapTargets::default();
            let grid = SnapSettings { grid: Some(5.0), objects: true, ..Default::default() };
            let s = snap_point_tracked(Point::new(100.0, 33.4), &grid, &none, &tracked, 2.0);
            assert_eq!(s.point.y, 33.0);
            let off = SnapSettings { grid: None, objects: false, ..Default::default() };
            let s = snap_point_tracked(Point::new(100.0, 33.4), &off, &none, &tracked, 2.0);
            assert_eq!(s.point.y, 33.4);
        }

        #[test]
        fn tracking_lines_are_dashed_and_cross_the_whole_view_with_a_plus_at_the_point() {
            let hit = Target { at: 30.0, kind: TargetKind::Tracked, span: Some((40.0, 40.0)) };
            let a = alignment(false, &hit, Rect::new(90.0, 30.0, 90.0, 30.0), Rect::new(0.0, 0.0, 400.0, 300.0));
            assert!(a.dashed);
            assert_eq!(a.line, (Point::new(0.0, 30.0), Point::new(400.0, 30.0)));
            assert_eq!(a.marks, [(Mark::Plus, Point::new(40.0, 30.0))]);
            assert_eq!(label(Some(&hit), None), Some("Tracking"));
        }

        #[test]
        fn a_diagonal_through_a_tracked_point_pulls_the_cursor_onto_it() {
            let tracked = [Point::new(100.0, 100.0)];
            let none = SnapTargets::default();
            let s = snap_point_tracked(Point::new(250.0, 251.4), &SnapSettings::default(), &none, &tracked, 3.0);
            assert!((s.point.x - 250.7).abs() < 1e-9 && (s.point.y - 250.7).abs() < 1e-9, "{:?}", s.point);
            assert_eq!(s.diagonal.map(|d| d.degrees), Some(45.0));
            let s = snap_point_tracked(Point::new(250.0, -49.0), &SnapSettings::default(), &none, &tracked, 3.0);
            assert_eq!(s.diagonal.map(|d| d.degrees), Some(135.0), "up and to the right");
            assert!(s.point.distance(Point::new(250.0, -50.0)) < 1.5, "{:?}", s.point);
            // Off every angle: untouched.
            let s = snap_point_tracked(Point::new(250.0, 180.0), &SnapSettings::default(), &none, &tracked, 3.0);
            assert_eq!((s.point, s.diagonal), (Point::new(250.0, 180.0), None));
        }

        #[test]
        fn thirty_and_sixty_degree_lines_are_offered_too() {
            let tracked = [Point::new(100.0, 100.0)];
            let none = SnapTargets::default();
            let at = |deg: f64, len: f64, nudge: f64| {
                let r = deg.to_radians();
                Point::new(100.0 + len * r.cos(), 100.0 + len * r.sin() + nudge)
            };
            for deg in [30.0, 60.0, 120.0, 150.0] {
                let s = snap_point_tracked(at(deg, 200.0, 1.0), &SnapSettings::default(), &none, &tracked, 3.0);
                assert_eq!(s.diagonal.map(|d| d.degrees), Some(deg), "{deg}");
                let want = at(deg, 200.0, 0.0);
                assert!(s.point.distance(want) < 1.5, "{deg}: {:?} vs {want:?}", s.point);
            }
        }

        #[test]
        fn a_moving_box_is_nudged_by_whichever_corner_is_nearest_a_tracking_line() {
            let tracked = [Point::new(100.0, 100.0)];
            // A 20 x 20 box whose top-left is at (150, 151.3): its corner is 1.3 mm off the 45-degree line.
            let keys =
                [Point::new(150.0, 151.3), Point::new(170.0, 151.3), Point::new(160.0, 161.3), Point::new(150.0, 171.3), Point::new(170.0, 171.3)];
            let n = track_translation(&keys, &tracked, TrackAngles::default(), (false, false), 3.0).unwrap();
            assert!(n.diagonal.is_some_and(|d| d.degrees == 45.0), "{n:?}");
            assert!((n.by.0 + n.by.1).abs() < 1.5 && n.by.1 < 0.0, "moves up/left onto the line: {n:?}");
            // Row tracking: the box's top edge 1 mm below the row through the point.
            let n = track_translation(&[Point::new(300.0, 101.0)], &tracked, TrackAngles::default(), (false, false), 3.0).unwrap();
            assert_eq!((n.by, n.y.map(|t| t.at)), ((0.0, -1.0), Some(100.0)));
        }

        #[test]
        fn a_locked_axis_is_left_alone_and_angles_wait_for_both_to_be_free() {
            let tracked = [Point::new(100.0, 100.0)];
            let near_diagonal = [Point::new(150.0, 151.3)];
            assert_eq!(track_translation(&near_diagonal, &tracked, TrackAngles::default(), (true, false), 3.0), None, "no angled line with x locked");
            assert_eq!(track_translation(&near_diagonal, &tracked, TrackAngles::default(), (true, true), 3.0), None);
            // A key on the row through the point, y locked: it would move y, so nothing happens.
            assert_eq!(track_translation(&[Point::new(300.0, 101.0)], &tracked, TrackAngles::default(), (false, true), 3.0), None);
            // A key near the column: x is free and gets pulled.
            let n = track_translation(&[Point::new(101.5, 300.0)], &tracked, TrackAngles::default(), (false, true), 3.0).unwrap();
            assert_eq!(n.by, (-1.5, 0.0));
            assert_eq!(track_translation(&[], &tracked, TrackAngles::default(), (false, false), 3.0), None);
            assert_eq!(track_translation(&near_diagonal, &[], TrackAngles::default(), (false, false), 3.0), None);
            assert_eq!(track_translation(&[Point::new(f64::NAN, 1.0)], &tracked, TrackAngles::default(), (false, false), 3.0), None);
        }

        #[test]
        fn the_angle_preset_decides_which_angled_lines_pull() {
            let none = SnapTargets::default();
            let tracked = [Point::new(100.0, 100.0)];
            let with = |angles| SnapSettings { grid: None, objects: true, angles };
            let near = |deg: f64| {
                let r = deg.to_radians();
                Point::new(100.0 + 200.0 * r.cos(), 100.0 + 200.0 * r.sin() + 1.0)
            };
            for (angles, deg, hit) in [
                (TrackAngles::RowColumn, 45.0, false),
                (TrackAngles::Diagonals, 45.0, true),
                (TrackAngles::Diagonals, 30.0, false),
                (TrackAngles::ThirtySixty, 30.0, true),
                (TrackAngles::ThirtySixty, 15.0, false),
                (TrackAngles::Fifteen, 15.0, true),
                (TrackAngles::Fifteen, 165.0, true),
            ] {
                let s = snap_point_tracked(near(deg), &with(angles), &none, &tracked, 3.0);
                assert_eq!(s.diagonal.is_some(), hit, "{angles:?} at {deg}");
            }
            for a in TrackAngles::ALL {
                assert!(a.degrees().iter().all(|d| d.is_finite() && *d > 0.0 && *d < 180.0) && !a.label().is_empty());
            }
        }

        #[test]
        fn a_diagonal_meets_a_locked_column_exactly() {
            let tracked = [Point::new(100.0, 100.0)];
            let mut t = SnapTargets::default();
            t.add_guide(true, 250.0);
            let s = snap_point_tracked(Point::new(249.0, 252.0), &SnapSettings::default(), &t, &tracked, 3.0);
            assert!(s.point.distance(Point::new(250.0, 250.0)) < 1e-9, "the guide's column and the diagonal cross there: {:?}", s.point);
            assert_eq!(s.point.x, 250.0, "the column stays exact");
            assert!(s.x_line.is_some() && s.diagonal.is_some());
            // Both axes already locked: nothing left for a diagonal to do.
            t.add_guide(false, 252.0);
            let s = snap_point_tracked(Point::new(249.0, 252.0), &SnapSettings::default(), &t, &tracked, 3.0);
            assert_eq!(s.diagonal, None);
        }

        #[test]
        fn right_on_the_acquired_point_there_is_no_diagonal_to_follow() {
            let tracked = [Point::new(100.0, 100.0)];
            let none = SnapTargets::default();
            let s = snap_point_tracked(
                Point::new(101.0, 100.5),
                &SnapSettings { grid: None, objects: true, angles: TrackAngles::default() },
                &none,
                &tracked,
                3.0,
            );
            assert_eq!(s.diagonal, None);
        }

        #[test]
        fn bad_numbers_are_ignored() {
            let none = SnapTargets::default();
            let s = snap_point_tracked(Point::new(f64::NAN, 1.0), &SnapSettings::default(), &none, &[Point::new(f64::NAN, 1.0)], 2.0);
            assert!(s.x_line.is_none());
            let s = snap_point_tracked(Point::new(1.0, 1.0), &SnapSettings::default(), &none, &[Point::new(f64::INFINITY, f64::NAN)], 2.0);
            assert_eq!(s.point, Point::new(1.0, 1.0));
        }
    }

    mod vertex_tests {
        use kurbo::{Affine, BezPath};

        use super::*;

        #[test]
        fn rectangles_give_corners_and_edge_midpoints_in_workspace_space() {
            let path = laserfoundry_geom::rect_path(0.0, 0.0, 10.0, 4.0);
            let pts = path_points(&path, Affine::translate((100.0, 50.0)), 100);
            for want in [Point::new(100.0, 50.0), Point::new(110.0, 54.0), Point::new(105.0, 50.0), Point::new(110.0, 52.0)] {
                assert!(pts.iter().any(|(p, _)| p.distance(want) < 1e-9), "{want:?} in {pts:?}");
            }
        }

        #[test]
        fn curves_contribute_endpoints_only_and_the_cap_and_bad_numbers_are_respected() {
            let mut curve = BezPath::new();
            curve.move_to((0.0, 0.0));
            curve.curve_to((5.0, 30.0), (15.0, 30.0), (20.0, 0.0));
            let pts = path_points(&curve, Affine::IDENTITY, 100);
            assert_eq!(pts, vec![(Point::new(0.0, 0.0), TargetKind::Endpoint), (Point::new(20.0, 0.0), TargetKind::Endpoint)], "no control points");
            let big = laserfoundry_geom::rect_path(0.0, 0.0, 10.0, 10.0);
            assert_eq!(path_points(&big, Affine::IDENTITY, 3).len(), 3);
            assert!(path_points(&big, Affine::IDENTITY, 0).is_empty());
            assert!(path_points(&big, Affine::scale(f64::NAN), 100).is_empty(), "non-finite results are dropped");
        }

        #[test]
        fn vertices_attract_on_both_axes() {
            let mut t = SnapTargets::default();
            t.add_point(Point::new(33.0, 44.0), TargetKind::Endpoint);
            let s = snap_point(Point::new(32.0, 45.0), &SnapSettings::default(), &t, 2.0);
            assert_eq!(s.point, Point::new(33.0, 44.0));
        }
    }

    mod point_tests {
        use super::*;

        #[test]
        fn nearby_edges_and_centres_attract_and_report_their_line() {
            let s = snap_point(Point::new(101.2, 69.0), &SnapSettings::default(), &targets(), 2.0);
            assert_eq!(s.point, Point::new(100.0, 70.0), "left edge, and the centre line y=70");
            assert_eq!((s.x_line.map(|t| t.at), s.y_line.map(|t| t.at)), (Some(100.0), Some(70.0)));
        }

        #[test]
        fn far_points_are_left_alone_and_axes_snap_independently() {
            let p = Point::new(124.5, 69.5);
            let s = snap_point(p, &SnapSettings::default(), &targets(), 2.0);
            assert_eq!(s.point, Point::new(124.5, 70.0), "x is over 4 mm from every line but y is on the centre line");
            assert_eq!(s.x_line, None);
        }

        #[test]
        fn the_grid_applies_when_no_object_is_close_and_objects_win_when_one_is() {
            let on = SnapSettings { grid: Some(5.0), objects: true, ..Default::default() };
            assert_eq!(snap_point(Point::new(12.4, 17.6), &on, &targets(), 2.0).point, Point::new(10.0, 20.0));
            assert_eq!(snap_point(Point::new(101.0, 17.6), &on, &targets(), 2.0).point, Point::new(100.0, 20.0), "object beats grid on x");
            let grid_only = SnapSettings { grid: Some(5.0), objects: false, ..Default::default() };
            assert_eq!(snap_point(Point::new(101.0, 70.4), &grid_only, &targets(), 2.0).point, Point::new(100.0, 70.0));
            let off = SnapSettings { grid: None, objects: false, ..Default::default() };
            assert_eq!(snap_point(Point::new(101.0, 70.4), &off, &targets(), 2.0).point, Point::new(101.0, 70.4));
        }

        #[test]
        fn hostile_values_pass_through_or_are_ignored() {
            let s = snap_point(
                Point::new(f64::NAN, f64::INFINITY),
                &SnapSettings { grid: Some(5.0), objects: true, ..Default::default() },
                &targets(),
                2.0,
            );
            assert!(s.point.x.is_nan() && s.point.y.is_infinite());
            let bad_grid = SnapSettings { grid: Some(0.0), objects: false, ..Default::default() };
            assert_eq!(snap_point(Point::new(3.3, 4.4), &bad_grid, &targets(), 2.0).point, Point::new(3.3, 4.4));
            let nan_grid = SnapSettings { grid: Some(f64::NAN), objects: false, ..Default::default() };
            assert_eq!(snap_point(Point::new(3.3, 4.4), &nan_grid, &targets(), 2.0).point, Point::new(3.3, 4.4));
            let mut t = SnapTargets::default();
            t.xs.push(Target { at: f64::NAN, kind: TargetKind::Edge, span: None });
            assert_eq!(snap_point(Point::new(1.0, 1.0), &SnapSettings::default(), &t, 2.0).point, Point::new(1.0, 1.0));
        }
    }

    mod translation_tests {
        use super::*;

        fn keys(r: Rect) -> SnapTargets {
            let mut k = SnapTargets::default();
            k.add_rect(r);
            k
        }

        #[test]
        fn a_moving_selection_locks_its_nearest_edge_to_a_target() {
            // A 20x20 box at (60,50) dragged right by 18.5: its right edge (80+18.5=98.5) is 1.5 from x=100? No: its
            // left edge ends at 78.5 and right at 98.5; the right edge is within 2 of the target at x=100.
            let k = keys(Rect::new(60.0, 50.0, 80.0, 70.0));
            let ((dx, dy), xl, _) = snap_translation((18.5, 0.0), &k, &SnapSettings::default(), &targets(), 2.0);
            assert!((dx - 20.0).abs() < 1e-9 && dy == 0.0, "{dx}");
            assert_eq!(xl.map(|t| t.at), Some(100.0));
        }

        #[test]
        fn the_closest_alignment_wins_among_several() {
            let k = keys(Rect::new(0.0, 0.0, 40.0, 40.0)); // same size as the target: edges can align two ways
            let ((dx, _), _, _) = snap_translation((100.7, 0.0), &k, &SnapSettings::default(), &targets(), 2.0);
            assert!((dx - 100.0).abs() < 1e-9, "left edge to 100 (0.7 away) beats right edge to 140 (0.7 away on the other side, equal) — got {dx}");
        }

        #[test]
        fn grid_snapping_moves_the_selections_corner_onto_the_grid() {
            let k = keys(Rect::new(3.0, 3.0, 13.0, 13.0));
            let grid = SnapSettings { grid: Some(5.0), objects: false, ..Default::default() };
            let ((dx, dy), _, _) = snap_translation((9.0, 4.0), &k, &grid, &targets(), 2.0);
            assert!((3.0 + dx - 10.0).abs() < 1e-9 && (3.0 + dy - 5.0).abs() < 1e-9, "{dx} {dy}");
        }

        #[test]
        fn nothing_in_range_changes_nothing_and_nan_is_untouched() {
            let k = keys(Rect::new(0.0, 0.0, 5.0, 5.0));
            assert_eq!(snap_translation((13.0, 7.0), &k, &SnapSettings::default(), &targets(), 2.0).0, (13.0, 7.0));
            let ((dx, _), _, _) = snap_translation((f64::NAN, 0.0), &k, &SnapSettings::default(), &targets(), 2.0);
            assert!(dx.is_nan());
            assert_eq!(
                snap_translation(
                    (1.0, 1.0),
                    &SnapTargets::default(),
                    &SnapSettings { grid: Some(5.0), objects: true, ..Default::default() },
                    &targets(),
                    2.0
                )
                .0,
                (1.0, 1.0),
                "no keys, nothing to snap"
            );
        }
    }

    mod constraint_tests {
        use super::*;

        #[test]
        fn square_constraint_keeps_direction_and_uses_the_longer_side() {
            assert_eq!(constrain_square(Point::new(10.0, 10.0), Point::new(40.0, 25.0)), Point::new(40.0, 40.0));
            assert_eq!(constrain_square(Point::new(10.0, 10.0), Point::new(-5.0, 12.0)), Point::new(-5.0, 25.0));
            assert_eq!(constrain_square(Point::new(10.0, 10.0), Point::new(10.0, 10.0)), Point::new(10.0, 10.0));
        }

        #[test]
        fn angle_constraint_snaps_to_multiples_and_keeps_length() {
            let end = constrain_angle(Point::new(0.0, 0.0), Point::new(10.0, 0.8), 15.0);
            assert!((end.x.hypot(end.y) - 10.0f64.hypot(0.8)).abs() < 1e-9);
            assert!(end.y.abs() < 1e-9, "4.6 degrees rounds to 0: {end:?}");
            let up = constrain_angle(Point::new(0.0, 0.0), Point::new(10.0, 1.5), 15.0);
            assert!((up.y.atan2(up.x).to_degrees() - 15.0).abs() < 1e-9, "8.5 degrees is nearer 15 than 0: {up:?}");
            let diag = constrain_angle(Point::new(0.0, 0.0), Point::new(9.0, 11.0), 45.0);
            assert!((diag.x - diag.y).abs() < 1e-9 && diag.x > 0.0);
            assert_eq!(constrain_angle(Point::new(1.0, 1.0), Point::new(1.0, 1.0), 15.0), Point::new(1.0, 1.0));
            assert_eq!(constrain_angle(Point::new(0.0, 0.0), Point::new(3.0, 4.0), 0.0), Point::new(3.0, 4.0));
        }
    }

    mod ruler_tests {
        use super::*;

        #[test]
        fn majors_are_one_two_five_steps_at_least_sixty_points_apart() {
            for zoom in [0.2, 0.5, 1.0, 2.0, 3.78, 10.0, 50.0, 400.0] {
                let (major, minor) = ruler_steps(zoom);
                assert!(major * zoom >= 59.9, "{zoom}: major {major}");
                let mantissa = major / 10f64.powf(major.log10().floor());
                assert!([1.0, 2.0, 5.0].iter().any(|m| (mantissa - m).abs() < 1e-6), "{zoom}: {major} is not 1-2-5");
                assert!(
                    minor <= major && (major / minor).fract().abs() < 1e-6 || (major / minor - (major / minor).round()).abs() < 1e-6,
                    "{zoom}: minor {minor} must divide major {major}"
                );
            }
        }

        #[test]
        fn hostile_zoom_has_a_sane_fallback() {
            for z in [0.0, -1.0, f64::NAN, f64::INFINITY] {
                assert_eq!(ruler_steps(z), (100.0, 10.0));
            }
        }
    }
}
