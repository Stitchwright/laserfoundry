//! Perforation: cutting an outline as a row of dashes with uncut gaps between them, which makes a
//! tear-off line or a part that can be snapped out.

use laserfoundry_geom::Point;

use crate::tabs::Piece;

/// Shortest cut or gap length accepted, in mm.
pub const MIN_DASH_MM: f64 = 0.05;
/// Most pieces one outline may be split into; longer outlines are cut solid instead.
const MAX_PIECES: f64 = 200_000.0;

/// Splits a polyline (closed ones include the closing edge) into alternating cut and skip pieces,
/// starting with a cut of `cut` mm. Returns `None` when the lengths are unusable or the outline
/// would need an absurd number of pieces.
pub fn perforate(points: &[Point], closed: bool, cut: f64, gap: f64) -> Option<Vec<Piece>> {
    if !(cut.is_finite() && gap.is_finite()) || cut < MIN_DASH_MM || gap < MIN_DASH_MM || points.len() < 2 {
        return None;
    }
    let mut segments: Vec<(Point, Point)> = points.windows(2).map(|w| (w[0], w[1])).collect();
    if closed && let (Some(first), Some(last)) = (points.first(), points.last()) {
        segments.push((*last, *first));
    }
    let total: f64 = segments.iter().map(|(a, b)| a.distance(*b)).sum();
    if !total.is_finite() || total / (cut + gap) * 2.0 > MAX_PIECES {
        return None;
    }
    let mut pieces = Vec::new();
    let mut on = true;
    let mut remaining = cut;
    let mut current = vec![*points.first()?];
    for (a, b) in segments {
        let len = a.distance(b);
        let mut pos = 0.0;
        while len > 0.0 && pos < len {
            let take = remaining.min(len - pos);
            pos += take;
            remaining -= take;
            let p = a.lerp(b, (pos / len).clamp(0.0, 1.0));
            current.push(p);
            if remaining <= 1e-9 {
                let next = vec![p];
                pieces.push(Piece { points: std::mem::replace(&mut current, next), on });
                on = !on;
                remaining = if on { cut } else { gap };
            }
        }
    }
    if current.len() > 1 {
        pieces.push(Piece { points: current, on });
    }
    Some(pieces)
}
