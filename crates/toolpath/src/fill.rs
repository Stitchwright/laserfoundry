use laserfoundry_geom::{Affine, Point, Polyline, bounds_of_polylines};

/// Even-odd scanline fill of every closed polyline together. Returns segments (start, end);
/// with `bidirectional` every other row is reversed so the head zig-zags.
pub fn fill_segments(lines: &[Polyline], interval: f64, angle_deg: f64, bidirectional: bool) -> Vec<(Point, Point)> {
    if !(interval.is_finite() && interval > 0.0) || !angle_deg.is_finite() {
        return Vec::new();
    }
    let rot = Affine::rotate(-angle_deg.to_radians());
    let rotated: Vec<Polyline> = lines.iter().filter(|l| l.closed && l.points.len() > 2).map(|l| l.transformed(rot)).collect();
    let Some(b) = bounds_of_polylines(&rotated) else { return Vec::new() };
    // Bound the row count so a tiny interval on a huge shape cannot run away.
    let rows = ((b.y1 - b.y0) / interval).ceil();
    if !rows.is_finite() || rows > 2_000_000.0 {
        return Vec::new();
    }
    let back = rot.inverse();
    let mut out = Vec::new();
    for row in 0..rows as usize {
        let y = b.y0 + interval * (row as f64 + 0.5);
        let mut xs = crossings(&rotated, y);
        xs.sort_by(f64::total_cmp);
        let reverse = bidirectional && row % 2 == 1;
        let mut spans: Vec<(f64, f64)> = xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect();
        if reverse {
            spans.reverse();
        }
        for (x0, x1) in spans {
            let (a, c) = if reverse { (x1, x0) } else { (x0, x1) };
            out.push((back * Point::new(a, y), back * Point::new(c, y)));
        }
    }
    out
}

fn crossings(lines: &[Polyline], y: f64) -> Vec<f64> {
    let mut xs = Vec::new();
    for l in lines {
        let n = l.points.len();
        for i in 0..n {
            let (Some(a), Some(b)) = (l.points.get(i), l.points.get((i + 1) % n)) else { continue };
            if (a.y > y) != (b.y > y) {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
    }
    xs
}
