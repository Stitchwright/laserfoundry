//! Turning a picture into outlines: pixels darker than a threshold become filled regions whose
//! boundaries are traced along pixel edges and then simplified into straight segments.

use std::collections::HashMap;

use laserfoundry_geom::{Point, Polyline};

/// Most pixels that will be traced.
pub const MAX_TRACE_PIXELS: usize = 16_000_000;
/// Most boundary edges followed; a noisy picture hits this long before memory matters.
const MAX_EDGES: usize = 4_000_000;
/// Most loops returned.
pub const MAX_LOOPS: usize = 50_000;

/// Boundary loops of the pixels for which `inside(x, y)` holds, in pixel units (the top-left of
/// pixel (0, 0) is the origin). Outer boundaries run clockwise on screen and holes counter-clockwise,
/// so filling the result with the even-odd rule gives the shape. Loops enclosing less than
/// `min_area` square pixels are dropped, and the rest are simplified to within `tolerance` pixels.
/// Returns `None` when the picture is too large or too busy to trace.
pub fn trace(w: usize, h: usize, inside: impl Fn(usize, usize) -> bool, tolerance: f64, min_area: f64) -> Option<Vec<Polyline>> {
    let pixels = w.checked_mul(h)?;
    if w == 0 || h == 0 || pixels > MAX_TRACE_PIXELS || w > i32::MAX as usize / 2 || h > i32::MAX as usize / 2 {
        return None;
    }
    let tolerance = if tolerance.is_finite() { tolerance.clamp(0.0, 50.0) } else { 1.0 };
    let min_area = if min_area.is_finite() { min_area.max(0.0) } else { 0.0 };
    let get = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && inside(x as usize, y as usize);

    // Directions: 0 = +x, 1 = +y, 2 = -x, 3 = -y (y points down, so turning "right" is d + 1).
    let mut out_edges: HashMap<(i32, i32), [bool; 4]> = HashMap::new();
    let mut edges = 0usize;
    let mut add = |x: i32, y: i32, d: usize| {
        out_edges.entry((x, y)).or_insert([false; 4])[d] = true;
    };
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !get(x, y) {
                continue;
            }
            let (xi, yi) = (x as i32, y as i32);
            if !get(x, y - 1) {
                add(xi, yi, 0);
                edges += 1;
            }
            if !get(x + 1, y) {
                add(xi + 1, yi, 1);
                edges += 1;
            }
            if !get(x, y + 1) {
                add(xi + 1, yi + 1, 2);
                edges += 1;
            }
            if !get(x - 1, y) {
                add(xi, yi + 1, 3);
                edges += 1;
            }
            if edges > MAX_EDGES {
                return None;
            }
        }
    }

    let mut loops = Vec::new();
    // A fixed order (top row first) keeps the result the same from run to run.
    let mut starts: Vec<(i32, i32)> = out_edges.keys().copied().collect();
    starts.sort_by_key(|&(x, y)| (y, x));
    for start in starts {
        while let Some(d0) = out_edges.get(&start).and_then(|e| e.iter().position(|b| *b)) {
            let mut ring = Vec::new();
            let (mut at, mut d) = (start, d0);
            loop {
                if let Some(e) = out_edges.get_mut(&at) {
                    e[d] = false;
                }
                ring.push(Point::new(f64::from(at.0), f64::from(at.1)));
                at = match d {
                    0 => (at.0 + 1, at.1),
                    1 => (at.0, at.1 + 1),
                    2 => (at.0 - 1, at.1),
                    _ => (at.0, at.1 - 1),
                };
                if at == start {
                    break; // closed
                }
                let Some(avail) = out_edges.get(&at) else { break };
                // Prefer turning right, then straight, then left, so diagonal neighbours join up.
                let Some(next) = [(d + 1) % 4, d, (d + 3) % 4].into_iter().find(|n| avail[*n]) else { break };
                d = next;
                if ring.len() > MAX_EDGES {
                    return None;
                }
            }
            if ring.len() >= 4 {
                loops.push(ring);
            }
            if loops.len() > MAX_LOOPS {
                return None;
            }
        }
    }

    let mut result = Vec::new();
    for ring in loops {
        if area(&ring).abs() < min_area {
            continue;
        }
        let pts = simplify_closed(&ring, tolerance);
        if pts.len() >= 3 {
            result.push(Polyline::new(pts, true));
        }
    }
    Some(result)
}

fn area(pts: &[Point]) -> f64 {
    let mut s = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        s += a.x * b.y - b.x * a.y;
    }
    s / 2.0
}

/// Douglas-Peucker for a closed ring: split at the two points farthest apart, simplify each half.
fn simplify_closed(ring: &[Point], tol: f64) -> Vec<Point> {
    let ring = drop_collinear(ring);
    if ring.len() < 4 || tol <= 0.0 {
        return ring;
    }
    let a = 0;
    let b = ring.iter().enumerate().max_by(|x, y| ring[a].distance(*x.1).total_cmp(&ring[a].distance(*y.1))).map_or(ring.len() / 2, |(i, _)| i);
    let first: Vec<Point> = ring.get(a..=b).map(<[Point]>::to_vec).unwrap_or_default();
    let mut second: Vec<Point> = ring.get(b..).map(<[Point]>::to_vec).unwrap_or_default();
    second.push(ring[0]);
    let mut keep = rdp(&first, tol);
    keep.pop(); // the split point is the start of the second half
    keep.extend(rdp(&second, tol));
    keep.pop(); // the closing point repeats the first
    keep
}

fn rdp(pts: &[Point], tol: f64) -> Vec<Point> {
    let (Some(first), Some(last)) = (pts.first(), pts.last()) else { return Vec::new() };
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let mut worst = (0usize, 0.0f64);
    for (i, p) in pts.iter().enumerate().skip(1).take(pts.len() - 2) {
        let d = laserfoundry_geom::distance_to_segment(*p, *first, *last);
        if d > worst.1 {
            worst = (i, d);
        }
    }
    if worst.1 <= tol {
        return vec![*first, *last];
    }
    let mut left = rdp(pts.get(..=worst.0).unwrap_or_default(), tol);
    left.pop();
    left.extend(rdp(pts.get(worst.0..).unwrap_or_default(), tol));
    left
}

/// Removes vertices that lie on a straight run (the staircase of a long edge).
fn drop_collinear(ring: &[Point]) -> Vec<Point> {
    let n = ring.len();
    (0..n)
        .filter(|i| {
            let (a, b, c) = (ring[(i + n - 1) % n], ring[*i], ring[(i + 1) % n]);
            (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x) != 0.0
        })
        .map(|i| ring[i])
        .collect()
}
