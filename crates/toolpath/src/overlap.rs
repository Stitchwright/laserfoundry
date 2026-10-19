use laserfoundry_geom::{Point, Polyline};

/// Points closer than this (mm) to a line lie on it; overlaps shorter than this are ignored.
const ON_LINE_MM: f64 = 1e-4;
/// The comparison is quadratic, so very large jobs are left as they are.
const MAX_SEGMENTS: usize = 3000;

type Segment = (Point, Point);

/// Removes the stretches of straight edges that lie on top of an earlier edge (two shapes sharing a
/// side, a line drawn twice with different end points), so that the laser cuts each stretch once.
/// Polylines that overlap nothing come back untouched; a trimmed one is split into open pieces.
/// Non-finite coordinates and very large jobs are left alone.
pub fn remove_overlaps(lines: Vec<Polyline>) -> Vec<Polyline> {
    let total: usize = lines.iter().map(|l| l.points.len()).sum();
    if total > MAX_SEGMENTS || lines.iter().flat_map(|l| &l.points).any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return lines;
    }
    let mut kept: Vec<Segment> = Vec::new();
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let segs = segments(&line);
        let mut pieces: Vec<Segment> = Vec::new();
        let mut trimmed = false;
        for &(a, b) in &segs {
            let spans = uncovered(a, b, &kept);
            if spans.len() != 1 || spans.first() != Some(&(0.0, 1.0)) {
                trimmed = true;
            }
            for (t0, t1) in spans {
                let (p, q) = (lerp(a, b, t0), lerp(a, b, t1));
                pieces.push((p, q));
                kept.push((p, q));
            }
        }
        if !trimmed {
            out.push(line);
            continue;
        }
        let mut current: Vec<Point> = Vec::new();
        for (p, q) in pieces {
            if current.last() != Some(&p) {
                if current.len() > 1 {
                    out.push(Polyline::new(std::mem::take(&mut current), false));
                }
                current = vec![p];
            }
            current.push(q);
        }
        if current.len() > 1 {
            out.push(Polyline::new(current, false));
        }
    }
    out
}

fn segments(line: &Polyline) -> Vec<Segment> {
    let mut segs: Vec<Segment> = line.points.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))).collect();
    if line.closed
        && let (Some(&last), Some(&first)) = (line.points.last(), line.points.first())
    {
        segs.push((last, first));
    }
    segs
}

/// The parts of a-b (as ranges of 0..1 along it) not covered by any collinear kept segment.
fn uncovered(a: Point, b: Point, kept: &[Segment]) -> Vec<(f64, f64)> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    let mut spans = vec![(0.0, 1.0)];
    if len <= ON_LINE_MM {
        return spans;
    }
    let min_span = ON_LINE_MM / len;
    for &(p, q) in kept {
        let dist = |r: Point| ((r.x - a.x) * dy - (r.y - a.y) * dx).abs() / len;
        if dist(p) > ON_LINE_MM || dist(q) > ON_LINE_MM {
            continue;
        }
        let along = |r: Point| ((r.x - a.x) * dx + (r.y - a.y) * dy) / (len * len);
        let (lo, hi) = (along(p).min(along(q)).max(0.0), along(p).max(along(q)).min(1.0));
        if hi - lo <= min_span {
            continue;
        }
        spans = spans
            .into_iter()
            .flat_map(|(s, e)| {
                let mut parts = Vec::new();
                if lo > s + min_span {
                    parts.push((s, lo.min(e)));
                }
                if hi < e - min_span {
                    parts.push((hi.max(s), e));
                }
                if lo <= s + min_span && hi >= e - min_span {
                    return Vec::new();
                }
                if parts.is_empty() { vec![(s, e)] } else { parts }
            })
            .collect();
    }
    spans
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    if t <= 0.0 {
        a
    } else if t >= 1.0 {
        b
    } else {
        Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
    }
}
