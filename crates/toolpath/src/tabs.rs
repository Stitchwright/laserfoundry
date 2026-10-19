//! Tabs (bridges) and lead-ins for closed cut outlines.
//!
//! Tabs leave small uncut gaps so a part stays attached to the sheet; lead-ins start the cut on
//! the scrap side of the line so the pierce mark does not land on the finished edge.

use laserfoundry_geom::{Point, Polyline, Vec2};

/// Most tabs one outline may get.
pub const MAX_TABS: u32 = 1000;

/// A stretch of an outline, to be cut (`on`) or skipped over with the laser off.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub points: Vec<Point>,
    pub on: bool,
}

/// A closed polyline measured by arc length.
struct Ring<'a> {
    pts: &'a [Point],
    /// Cumulative length at each vertex; `cum[i]` is the distance from vertex 0 to vertex i.
    cum: Vec<f64>,
    total: f64,
}

impl<'a> Ring<'a> {
    fn new(pts: &'a [Point]) -> Self {
        let mut cum = Vec::with_capacity(pts.len() + 1);
        let mut total = 0.0;
        cum.push(0.0);
        for i in 0..pts.len() {
            total += pts[i].distance(pts[(i + 1) % pts.len()]);
            cum.push(total);
        }
        Ring { pts, cum, total }
    }

    fn point_at(&self, s: f64) -> Point {
        let s = s.clamp(0.0, self.total);
        let i = self.cum.partition_point(|c| *c <= s).saturating_sub(1).min(self.pts.len() - 1);
        let (a, b) = (self.pts[i], self.pts[(i + 1) % self.pts.len()]);
        let seg = self.cum[i + 1] - self.cum[i];
        if seg <= 0.0 { a } else { a.lerp(b, (s - self.cum[i]) / seg) }
    }

    /// Points along the ring from arc length `s0` to `s1` (`s0 <= s1`), inclusive of both ends.
    fn slice(&self, s0: f64, s1: f64) -> Vec<Point> {
        let mut out = vec![self.point_at(s0)];
        for (i, c) in self.cum.iter().enumerate().take(self.pts.len()) {
            if *c > s0 && *c < s1 {
                out.push(self.pts[i]);
            }
        }
        out.push(self.point_at(s1));
        out
    }
}

/// Splits a closed outline into cut and skip pieces: `count` tabs of `width` mm, spread evenly and
/// centred between the equal divisions (so none sits on the start point). The pieces run in order
/// and end back at the start. Returns `None` when tabs do not fit (too small, or they would take
/// more than half the outline), in which case the outline should be cut whole.
pub fn split_for_tabs(outline: &Polyline, count: u32, width: f64) -> Option<Vec<Piece>> {
    if !outline.closed || outline.points.len() < 3 || count == 0 || !(width.is_finite() && width > 0.0) {
        return None;
    }
    let count = count.min(MAX_TABS);
    let ring = Ring::new(&outline.points);
    let step = ring.total / f64::from(count);
    if !ring.total.is_finite() || width * f64::from(count) > ring.total * 0.5 || width >= step {
        return None;
    }
    let mut pieces = Vec::with_capacity(count as usize * 2 + 1);
    let mut cursor = 0.0;
    for k in 0..count {
        let centre = (f64::from(k) + 0.5) * step;
        let (a, b) = (centre - width / 2.0, centre + width / 2.0);
        pieces.push(Piece { points: ring.slice(cursor, a), on: true });
        pieces.push(Piece { points: ring.slice(a, b), on: false });
        cursor = b;
    }
    pieces.push(Piece { points: ring.slice(cursor, ring.total), on: true });
    Some(pieces)
}

/// Nesting depth of every closed outline: how many other closed outlines contain it. Even depths
/// are material (outer outlines); odd depths are holes.
pub fn nesting_depths(lines: &[Polyline]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| match l.points.first() {
            Some(p) if l.closed => lines.iter().enumerate().filter(|(j, o)| *j != i && o.contains(*p)).count(),
            _ => 0,
        })
        .collect()
}

/// The same closed outline, restarted at the middle of its longest edge. A lead is perpendicular
/// to the path where it begins; at a corner that direction runs along the neighbouring edge, so
/// leads need a start point in the middle of an edge.
pub fn restart_mid_edge(outline: &Polyline) -> Polyline {
    let n = outline.points.len();
    if !outline.closed || n < 3 {
        return outline.clone();
    }
    let longest = (0..n)
        .max_by(|a, b| outline.points[*a].distance(outline.points[(a + 1) % n]).total_cmp(&outline.points[*b].distance(outline.points[(b + 1) % n])))
        .unwrap_or(0);
    let mid = outline.points[longest].midpoint(outline.points[(longest + 1) % n]);
    let mut points = vec![mid];
    points.extend((1..=n).map(|k| outline.points[(longest + k) % n]));
    Polyline::new(points, true)
}

/// A point `len` mm from the outline's start, off to the scrap side, for a lead-in or lead-out.
/// Scrap is outside an outer outline and inside a hole. `depth` is the outline's nesting depth.
pub fn lead_point(outline: &Polyline, depth: usize, len: f64) -> Option<Point> {
    if !(len.is_finite() && len > 0.0) || outline.points.len() < 3 {
        return None;
    }
    let start = *outline.points.first()?;
    let next = outline.points.iter().skip(1).find(|p| p.distance(start) > 1e-9)?;
    let tangent = (*next - start).normalize_or_zero();
    if tangent == Vec2::ZERO {
        return None;
    }
    let normal = Vec2::new(tangent.y, -tangent.x);
    let probe = 1e-3_f64.min(len / 10.0);
    let toward_hole_side = outline.contains(start + normal * probe);
    let want_inside = !depth.is_multiple_of(2);
    let n = if toward_hole_side == want_inside { normal } else { -normal };
    Some(start + n * len)
}

trait Normalize {
    fn normalize_or_zero(self) -> Vec2;
}

impl Normalize for Vec2 {
    fn normalize_or_zero(self) -> Vec2 {
        let len = self.hypot();
        if len > 1e-12 { self / len } else { Vec2::ZERO }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(s: f64) -> Polyline {
        Polyline::new(vec![Point::new(0.0, 0.0), Point::new(s, 0.0), Point::new(s, s), Point::new(0.0, s)], true)
    }

    fn length(points: &[Point]) -> f64 {
        points.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    mod tab_tests {
        use super::*;

        #[test]
        fn four_tabs_on_a_square_remove_exactly_their_width_each() {
            let pieces = split_for_tabs(&square(40.0), 4, 2.0).unwrap();
            let off: f64 = pieces.iter().filter(|p| !p.on).map(|p| length(&p.points)).sum();
            let on: f64 = pieces.iter().filter(|p| p.on).map(|p| length(&p.points)).sum();
            assert!((off - 8.0).abs() < 1e-9 && (on - 152.0).abs() < 1e-9, "{off} {on}");
            assert_eq!(pieces.iter().filter(|p| !p.on).count(), 4);
        }

        #[test]
        fn pieces_are_contiguous_start_and_end_at_the_outline_start_and_avoid_it() {
            let sq = square(40.0);
            let pieces = split_for_tabs(&sq, 3, 1.5).unwrap();
            assert_eq!(pieces.first().unwrap().points[0], sq.points[0]);
            assert!(pieces.last().unwrap().points.last().unwrap().distance(sq.points[0]) < 1e-9, "closes back at the start");
            for w in pieces.windows(2) {
                assert!(w[0].points.last().unwrap().distance(w[1].points[0]) < 1e-9, "no jump between pieces");
            }
            assert!(pieces.first().unwrap().on && pieces.last().unwrap().on, "the start is never inside a tab");
        }

        #[test]
        fn a_tab_that_straddles_a_corner_follows_it() {
            // One tab centred on a 40 mm square's perimeter midpoint (80 mm) sits exactly on a corner.
            let pieces = split_for_tabs(&square(40.0), 1, 4.0).unwrap();
            let tab = pieces.iter().find(|p| !p.on).unwrap();
            assert_eq!(tab.points.len(), 3, "start, the corner, end: {:?}", tab.points);
            assert!((length(&tab.points) - 4.0).abs() < 1e-9);
        }

        #[test]
        fn tabs_that_do_not_fit_are_declined() {
            assert!(split_for_tabs(&square(40.0), 4, 25.0).is_none(), "wider than a division");
            assert!(split_for_tabs(&square(4.0), 4, 2.5).is_none(), "more than half the outline");
            assert!(split_for_tabs(&square(4.0), 4, 2.0).is_some(), "exactly half is allowed");
            assert!(split_for_tabs(&square(40.0), 0, 2.0).is_none());
            for w in [0.0, -1.0, f64::NAN, f64::INFINITY] {
                assert!(split_for_tabs(&square(40.0), 4, w).is_none(), "{w}");
            }
            let open = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(9.0, 0.0), Point::new(9.0, 9.0)], false);
            assert!(split_for_tabs(&open, 2, 1.0).is_none());
        }

        #[test]
        fn absurd_counts_are_capped() {
            let pieces = split_for_tabs(&square(100000.0), u32::MAX, 0.1).unwrap();
            assert_eq!(pieces.iter().filter(|p| !p.on).count(), MAX_TABS as usize);
        }
    }

    mod lead_tests {
        use super::*;

        #[test]
        fn outer_outlines_lead_in_from_outside_and_holes_from_inside() {
            let sq = square(20.0); // start (0,0), travelling along +x
            let outer = lead_point(&sq, 0, 3.0).unwrap();
            assert!(!sq.contains(outer) && (outer.distance(sq.points[0]) - 3.0).abs() < 1e-9, "{outer:?}");
            let hole = lead_point(&sq, 1, 3.0).unwrap();
            assert!(sq.contains(hole), "a hole's scrap is inside it: {hole:?}");
            let island_in_a_hole = lead_point(&sq, 2, 3.0).unwrap();
            assert!(!sq.contains(island_in_a_hole), "even depth is material again");
        }

        #[test]
        fn the_lead_is_perpendicular_to_the_path_at_the_start() {
            let sq = square(20.0);
            let p = lead_point(&sq, 0, 2.0).unwrap();
            assert!(p.x.abs() < 1e-9, "start segment runs along x, so the lead is straight out in y: {p:?}");
            // Reverse the winding: it must still point away from the part.
            let mut rev = sq.clone();
            rev.points.reverse();
            let q = lead_point(&rev, 0, 2.0).unwrap();
            assert!(!rev.contains(q));
        }

        #[test]
        fn degenerate_outlines_and_lengths_give_no_lead() {
            assert!(lead_point(&square(10.0), 0, 0.0).is_none());
            assert!(lead_point(&square(10.0), 0, f64::NAN).is_none());
            let dot = Polyline::new(vec![Point::new(1.0, 1.0); 4], true);
            assert!(lead_point(&dot, 0, 2.0).is_none());
        }
    }

    #[test]
    fn restarting_mid_edge_keeps_the_shape_and_moves_the_start_off_the_corner() {
        let sq = square(20.0);
        let r = restart_mid_edge(&sq);
        assert_eq!(r.points.len(), 5);
        assert!(r.closed && !sq.points.contains(&r.points[0]), "the start is a new point, not a corner");
        let (a, b) = (length(&[r.points.as_slice(), &[r.points[0]]].concat()), length(&[sq.points.as_slice(), &[sq.points[0]]].concat()));
        assert!((a - b).abs() < 1e-9, "perimeter unchanged: {a} vs {b}");
        // From a mid-edge start the lead is straight out, clear of the corner edges.
        let lead = lead_point(&r, 0, 3.0).unwrap();
        assert!(!sq.contains(lead) && sq.distance_to(lead) > 2.9, "{lead:?}");
        // Degenerate input is returned as it was.
        let open = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0)], false);
        assert_eq!(restart_mid_edge(&open), open);
    }

    #[test]
    fn nesting_depths_count_containers() {
        let lines = [
            square(30.0),
            Polyline::new(vec![Point::new(5.0, 5.0), Point::new(15.0, 5.0), Point::new(15.0, 15.0), Point::new(5.0, 15.0)], true),
            Polyline::new(vec![Point::new(7.0, 7.0), Point::new(9.0, 7.0), Point::new(9.0, 9.0), Point::new(7.0, 9.0)], true),
        ];
        assert_eq!(nesting_depths(&lines), vec![0, 1, 2]);
    }
}
