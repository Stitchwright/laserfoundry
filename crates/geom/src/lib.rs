//! Geometry primitives on `kurbo`: polylines, flattening, bounds and hit-testing helpers.

pub mod along;
pub mod fit;
pub mod nodes;
pub mod register;

pub use kurbo::{Affine, BezPath, Point, Rect, Vec2};

use kurbo::PathEl;
use serde::{Deserialize, Serialize};

/// Largest number of points a single flattened path may produce; hostile paths are cut off here.
pub const MAX_FLATTENED_POINTS: usize = 2_000_000;
/// The finest tolerance flattening will use, as a fraction of the path's largest coordinate. Without it a curve a
/// million kilometres across asks for billions of segments before the output cap can apply.
const MIN_RELATIVE_TOLERANCE: f64 = 1e-7;
/// The tolerance to hand a curve builder (kurbo's arcs size themselves from `radius / tolerance`): `base`, but never finer
/// than [`MIN_RELATIVE_TOLERANCE`] of the curve's size, so a huge radius cannot ask for billions of segments.
pub fn curve_tolerance(size: f64, base: f64) -> f64 {
    if size.is_finite() { base.max(size.abs() * MIN_RELATIVE_TOLERANCE) } else { base }
}

/// The largest ellipse radius (mm) [`ellipse_path`] will draw.
pub const MAX_ELLIPSE_RADIUS: f64 = 1e12;

/// A flattened sub-path in some coordinate space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    pub points: Vec<Point>,
    pub closed: bool,
}

impl Polyline {
    pub fn new(points: Vec<Point>, closed: bool) -> Self {
        Self { points, closed }
    }

    pub fn transformed(&self, m: Affine) -> Polyline {
        Polyline { points: self.points.iter().map(|p| m * *p).collect(), closed: self.closed }
    }

    /// Total length, including the closing segment of closed polylines.
    pub fn length(&self) -> f64 {
        let mut total: f64 = self.points.windows(2).map(|w| w[0].distance(w[1])).sum();
        if self.closed
            && self.points.len() > 2
            && let (Some(first), Some(last)) = (self.points.first(), self.points.last())
        {
            total += first.distance(*last);
        }
        total
    }

    pub fn bounds(&self) -> Option<Rect> {
        bounds_of_points(self.points.iter().copied())
    }

    /// Even-odd containment test; open polylines contain nothing.
    pub fn contains(&self, p: Point) -> bool {
        if !self.closed || self.points.len() < 3 {
            return false;
        }
        let mut inside = false;
        let n = self.points.len();
        for i in 0..n {
            let a = self.points[i];
            let b = self.points[(i + n - 1) % n];
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
        }
        inside
    }

    /// Distance from `p` to the nearest edge.
    pub fn distance_to(&self, p: Point) -> f64 {
        let mut best = f64::INFINITY;
        for w in self.points.windows(2) {
            best = best.min(distance_to_segment(p, w[0], w[1]));
        }
        if self.closed
            && let (Some(first), Some(last)) = (self.points.first(), self.points.last())
        {
            best = best.min(distance_to_segment(p, *last, *first));
        }
        best
    }
}

/// Flattens a Bézier path into polylines within `tolerance` (same units as the path).
/// Non-finite coordinates are dropped and the output is capped at [`MAX_FLATTENED_POINTS`].
pub fn flatten(path: &BezPath, tolerance: f64) -> Vec<Polyline> {
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { 0.05 };
    let tolerance = curve_tolerance(extent(path), tolerance);
    let mut out: Vec<Polyline> = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    let mut total = 0usize;

    kurbo::flatten(path.iter(), tolerance, |el| {
        if total >= MAX_FLATTENED_POINTS {
            return;
        }
        match el {
            PathEl::MoveTo(p) => {
                finish(&mut out, &mut current, false);
                push_finite(&mut current, p);
            }
            PathEl::LineTo(p) => push_finite(&mut current, p),
            PathEl::ClosePath => finish(&mut out, &mut current, true),
            // `kurbo::flatten` only emits lines; curves never reach here.
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
        }
        total += 1;
    });
    finish(&mut out, &mut current, false);
    out
}

/// The largest finite coordinate magnitude in the path (0 if there is none).
fn extent(path: &BezPath) -> f64 {
    path.iter()
        .flat_map(|el| match el {
            PathEl::MoveTo(a) | PathEl::LineTo(a) => vec![a],
            PathEl::QuadTo(a, b) => vec![a, b],
            PathEl::CurveTo(a, b, c) => vec![a, b, c],
            PathEl::ClosePath => vec![],
        })
        .flat_map(|p| [p.x, p.y])
        .filter(|v| v.is_finite())
        .fold(0.0, |m: f64, v| m.max(v.abs()))
}

fn finish(out: &mut Vec<Polyline>, current: &mut Vec<Point>, closed: bool) {
    let mut pts = std::mem::take(current);
    if closed && pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() >= 2 {
        out.push(Polyline { points: pts, closed });
    }
}

fn push_finite(pts: &mut Vec<Point>, p: Point) {
    if p.x.is_finite() && p.y.is_finite() {
        pts.push(p);
    }
}

pub fn bounds_of_points(points: impl IntoIterator<Item = Point>) -> Option<Rect> {
    let mut it = points.into_iter();
    let first = it.next()?;
    let mut r = Rect::new(first.x, first.y, first.x, first.y);
    for p in it {
        r.x0 = r.x0.min(p.x);
        r.y0 = r.y0.min(p.y);
        r.x1 = r.x1.max(p.x);
        r.y1 = r.y1.max(p.y);
    }
    Some(r)
}

pub fn bounds_of_polylines(lines: &[Polyline]) -> Option<Rect> {
    lines.iter().filter_map(Polyline::bounds).reduce(|a, b| a.union(b))
}

pub fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let d = b - a;
    let len2 = d.hypot2();
    let t = if len2 == 0.0 { 0.0 } else { ((p - a).dot(d) / len2).clamp(0.0, 1.0) };
    p.distance(a + d * t)
}

/// Rectangle outline as a closed [`BezPath`].
pub fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + w, y));
    p.line_to((x + w, y + h));
    p.line_to((x, y + h));
    p.close_path();
    p
}

/// Ellipse outline (cubic arcs) as a closed [`BezPath`].
pub fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> BezPath {
    use kurbo::Shape;
    let (rx, ry) = (rx.abs(), ry.abs());
    // kurbo squares the radii (an SVD) and sizes the curve from them: past ~1e154 that overflows and it asks for an
    // unbounded number of segments. Nothing a laser can cut is anywhere near MAX_ELLIPSE_RADIUS, so give no outline.
    if !(rx.is_finite() && ry.is_finite() && cx.is_finite() && cy.is_finite()) || rx.max(ry) > MAX_ELLIPSE_RADIUS {
        return BezPath::new();
    }
    let tolerance = curve_tolerance(rx.max(ry), 0.01);
    let mut path = kurbo::Ellipse::new((cx, cy), (rx, ry), 0.0).to_path(tolerance);
    // kurbo leaves the outline open; without this the ellipse would not be a closed shape (no
    // fill, no containment, no boolean).
    if !matches!(path.elements().last(), Some(kurbo::PathEl::ClosePath)) {
        path.close_path();
    }
    path
}

/// Rectangle with rounded corners (radius clamped to half the shorter side), closed.
pub fn rounded_rect_path(x: f64, y: f64, w: f64, h: f64, radius: f64) -> BezPath {
    use kurbo::Shape;
    let r = if radius.is_finite() { radius.clamp(0.0, w.abs().min(h.abs()) / 2.0) } else { 0.0 };
    if r <= 0.0 {
        return rect_path(x, y, w, h);
    }
    let mut path = kurbo::RoundedRect::new(x, y, x + w, y + h, r).to_path(0.01);
    if !matches!(path.elements().last(), Some(kurbo::PathEl::ClosePath)) {
        path.close_path();
    }
    path
}

pub fn line_path(a: Point, b: Point) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    mod flatten_tests {
        use super::*;

        #[test]
        fn rectangle_flattens_to_one_closed_polyline() {
            let lines = flatten(&rect_path(0.0, 0.0, 10.0, 5.0), 0.1);
            assert_eq!(lines.len(), 1);
            assert!(lines[0].closed);
            assert_eq!(lines[0].points.len(), 4);
            assert!((lines[0].length() - 30.0).abs() < 1e-9);
        }

        #[test]
        fn circle_length_approximates_circumference() {
            let lines = flatten(&ellipse_path(0.0, 0.0, 10.0, 10.0), 0.01);
            let expected = 2.0 * std::f64::consts::PI * 10.0;
            assert!((lines[0].length() - expected).abs() < 0.2, "{}", lines[0].length());
        }

        #[test]
        fn ellipses_and_circles_are_closed_shapes() {
            let lines = flatten(&ellipse_path(5.0, 5.0, 4.0, 2.0), 0.01);
            assert_eq!(lines.len(), 1);
            assert!(lines[0].closed, "an ellipse must be a closed polyline");
            assert!(lines[0].contains(Point::new(5.0, 5.0)) && !lines[0].contains(Point::new(5.0, 9.0)));
        }

        #[test]
        fn rounded_rectangles_are_closed_clamped_and_cut_the_corners() {
            let sharp = flatten(&rect_path(0.0, 0.0, 20.0, 10.0), 0.01);
            let round = flatten(&rounded_rect_path(0.0, 0.0, 20.0, 10.0, 3.0), 0.01);
            assert!(round[0].closed);
            assert!(round[0].length() < sharp[0].length(), "corners are shorter than the right angle");
            assert!(!round[0].contains(Point::new(0.1, 0.1)) && round[0].contains(Point::new(10.0, 5.0)));
            // A radius larger than half the short side becomes a stadium, not a mess.
            let huge = flatten(&rounded_rect_path(0.0, 0.0, 20.0, 10.0, 99.0), 0.01);
            let b = bounds_of_polylines(&huge).unwrap();
            assert!((b.width() - 20.0).abs() < 1e-6 && (b.height() - 10.0).abs() < 1e-6);
            assert_eq!(flatten(&rounded_rect_path(0.0, 0.0, 5.0, 5.0, f64::NAN), 0.01)[0].points.len(), 4);
            assert_eq!(flatten(&rounded_rect_path(0.0, 0.0, 5.0, 5.0, -2.0), 0.01)[0].points.len(), 4);
        }

        #[test]
        fn a_curve_of_absurd_size_flattens_to_a_bounded_number_of_points() {
            // Regression: a DXF circle of radius 1e300 asked for billions of segments and killed the process.
            assert!(ellipse_path(0.0, 0.0, 1e300, 1e300).elements().is_empty());
            assert!(ellipse_path(0.0, 0.0, f64::INFINITY, 5.0).elements().is_empty());
            let lines = flatten(&ellipse_path(0.0, 0.0, 1e11, 1e11), 0.05);
            assert!(lines.iter().map(|l| l.points.len()).sum::<usize>() < 100_000);
            // A huge but representable curve in an ordinary path is also bounded by the relative tolerance.
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            p.curve_to((1e150, 0.0), (1e150, 1e150), (0.0, 1e150));
            assert!(flatten(&p, 0.05).iter().map(|l| l.points.len()).sum::<usize>() < 100_000);
            // Ordinary sizes keep the caller's tolerance.
            assert!(flatten(&ellipse_path(0.0, 0.0, 100.0, 100.0), 0.01)[0].points.len() > 50);
        }

        #[test]
        fn hostile_tolerance_and_nan_do_not_panic() {
            let mut p = BezPath::new();
            p.move_to((f64::NAN, 0.0));
            p.line_to((1.0, 1.0));
            p.line_to((2.0, 2.0));
            assert!(flatten(&p, f64::NAN).len() <= 1);
            assert!(flatten(&p, -1.0).len() <= 1);
        }
    }

    mod hit_tests {
        use super::*;

        #[test]
        fn containment_is_even_odd() {
            let lines = flatten(&rect_path(0.0, 0.0, 10.0, 10.0), 0.1);
            assert!(lines[0].contains(Point::new(5.0, 5.0)));
            assert!(!lines[0].contains(Point::new(15.0, 5.0)));
        }

        #[test]
        fn distance_to_edge() {
            let lines = flatten(&rect_path(0.0, 0.0, 10.0, 10.0), 0.1);
            assert!((lines[0].distance_to(Point::new(5.0, 13.0)) - 3.0).abs() < 1e-9);
        }
    }
}
