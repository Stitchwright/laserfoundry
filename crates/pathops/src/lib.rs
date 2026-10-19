//! Boolean combine and offset on flattened shapes (millimetres).
//!
//! A *region* is a list of closed polylines read with the even-odd rule, so a polyline nested
//! inside another is a hole. Heavy lifting is done by `i_overlay` (MIT OR Apache-2.0).

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{LineCap, LineJoin, OutlineStyle, StrokeStyle};
use laserfoundry_geom::{Point, Polyline};
use thiserror::Error;

/// Points accepted per operation.
pub const MAX_POINTS: usize = 1_000_000;
/// Largest offset distance, mm.
pub const MAX_OFFSET_MM: f64 = 10_000.0;
/// Coordinates beyond this are refused (the integer clipper works on a bounded range).
pub const MAX_COORD_MM: f64 = 1.0e6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Intersect,
    /// Subject minus clip.
    Difference,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Join {
    /// Rounded corners.
    Round,
    /// Sharp corners (limited: very acute corners are bevelled).
    Miter,
    /// Corners cut off flat.
    Bevel,
}

#[derive(Debug, Error, PartialEq)]
pub enum PathOpsError {
    #[error("shapes contain a non-finite or out-of-range coordinate")]
    BadCoordinate,
    #[error("too many points (limit {MAX_POINTS})")]
    TooLarge,
    #[error("offset distance must be a finite number within ±{MAX_OFFSET_MM} mm")]
    BadDistance,
    #[error("an open path needs a positive offset distance (it becomes the outline around the line)")]
    OpenPathNeedsPositiveOffset,
}

/// Combines two regions.
pub fn boolean(subject: &[Polyline], clip: &[Polyline], op: BoolOp) -> Result<Vec<Polyline>, PathOpsError> {
    let (s, c) = (contours(subject)?, contours(clip)?);
    check_size(&[&s, &c])?;
    let rule = match op {
        BoolOp::Union => OverlayRule::Union,
        BoolOp::Intersect => OverlayRule::Intersect,
        BoolOp::Difference => OverlayRule::Difference,
        BoolOp::Xor => OverlayRule::Xor,
    };
    Ok(to_polylines(s.overlay(&c, rule, FillRule::EvenOdd)))
}

/// Merges a region with itself: resolves overlaps and self-intersections under the even-odd rule.
pub fn normalize(region: &[Polyline]) -> Result<Vec<Polyline>, PathOpsError> {
    boolean(region, &[], BoolOp::Union)
}

/// Grows (`distance > 0`) or shrinks (`distance < 0`) a region. Closed polylines are offset as
/// shapes (holes shrink when the outside grows); open polylines become the outline of a stroke
/// `2 × distance` wide and need a positive distance. Results are independent closed polylines.
pub fn offset(lines: &[Polyline], distance: f64, join: Join) -> Result<Vec<Polyline>, PathOpsError> {
    if !(distance.is_finite() && distance.abs() <= MAX_OFFSET_MM) {
        return Err(PathOpsError::BadDistance);
    }
    let closed: Vec<Polyline> = lines.iter().filter(|l| l.closed).cloned().collect();
    let open: Vec<&Polyline> = lines.iter().filter(|l| !l.closed).collect();
    if !open.is_empty() && distance <= 0.0 {
        return Err(PathOpsError::OpenPathNeedsPositiveOffset);
    }
    let mut out: Vec<Polyline> = Vec::new();

    if !closed.is_empty() && distance != 0.0 {
        let region = contours(&closed)?;
        check_size(&[&region])?;
        // Resolve overlaps first so the offset sees proper shapes with holes.
        let shapes = region.overlay(&Vec::<Vec<[f64; 2]>>::new(), OverlayRule::Union, FillRule::EvenOdd);
        let style = OutlineStyle::new(distance).line_join(join_style(join, distance));
        for shape in shapes {
            out.extend(to_polylines(shape.outline(&style)));
        }
    } else {
        out.extend(normalize(&closed)?);
    }

    for l in open {
        let pts = points(l)?;
        let style =
            StrokeStyle::new(distance * 2.0).line_join(join_style(join, distance)).start_cap(LineCap::Round(0.1)).end_cap(LineCap::Round(0.1));
        out.extend(to_polylines(pts.stroke(style, false)));
    }
    Ok(out)
}

/// Largest number of open polylines `join` will chain (it searches near neighbours per end point).
pub const MAX_JOIN_POLYLINES: usize = 200_000;

/// Chains open polylines whose end points lie within `tolerance` of each other into longer
/// polylines, closing a chain whose two ends meet. Closed polylines pass through untouched.
/// This is how loose line segments (typical in DXF files) become cuttable, fillable shapes.
pub fn join(lines: &[Polyline], tolerance: f64) -> Result<Vec<Polyline>, PathOpsError> {
    if !(tolerance.is_finite() && (0.0..=MAX_OFFSET_MM).contains(&tolerance)) {
        return Err(PathOpsError::BadDistance);
    }
    let mut out: Vec<Polyline> = lines.iter().filter(|l| l.closed).cloned().collect();
    let open: Vec<&Polyline> = lines.iter().filter(|l| !l.closed && l.points.len() >= 2).collect();
    if open.len() > MAX_JOIN_POLYLINES {
        return Err(PathOpsError::TooLarge);
    }
    if open.iter().any(|l| l.points.iter().any(|p| !(p.x.is_finite() && p.y.is_finite()))) {
        return Err(PathOpsError::BadCoordinate);
    }
    let cell = tolerance.max(1e-6);
    let key = |p: Point| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    // End points by grid cell: (polyline index, is_start).
    let mut grid: std::collections::HashMap<(i64, i64), Vec<(usize, bool)>> = std::collections::HashMap::new();
    for (i, l) in open.iter().enumerate() {
        if let (Some(first), Some(last)) = (l.points.first(), l.points.last()) {
            grid.entry(key(*first)).or_default().push((i, true));
            grid.entry(key(*last)).or_default().push((i, false));
        }
    }
    let mut used = vec![false; open.len()];
    // Finds an unused polyline with an end point near `p`; returns its index and whether to read it forward.
    let find = |p: Point, used: &[bool]| -> Option<(usize, bool)> {
        let (cx, cy) = key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for (i, is_start) in grid.get(&(cx + dx, cy + dy)).into_iter().flatten() {
                    if used[*i] {
                        continue;
                    }
                    let end = if *is_start { open[*i].points.first() } else { open[*i].points.last() };
                    if end.is_some_and(|e| e.distance(p) <= tolerance) {
                        return Some((*i, *is_start));
                    }
                }
            }
        }
        None
    };
    for start in 0..open.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut chain: Vec<Point> = open[start].points.clone();
        // Grow at the tail, then at the head.
        while let Some(tail) = chain.last().copied()
            && let Some((i, forward)) = find(tail, &used)
        {
            used[i] = true;
            let pts = &open[i].points;
            if forward {
                chain.extend(pts.iter().skip(1));
            } else {
                chain.extend(pts.iter().rev().skip(1));
            }
        }
        while let Some(head) = chain.first().copied()
            && let Some((i, forward)) = find(head, &used)
        {
            used[i] = true;
            let pts = &open[i].points;
            let mut prefix: Vec<Point> = if forward { pts.iter().rev().cloned().collect() } else { pts.clone() };
            prefix.pop(); // the shared end point is already the chain's head
            prefix.extend(chain);
            chain = prefix;
        }
        let closes = chain.len() >= 4 && chain.first().zip(chain.last()).is_some_and(|(a, b)| a.distance(*b) <= tolerance);
        if closes {
            chain.pop();
        }
        out.push(Polyline::new(chain, closes));
    }
    Ok(out)
}

fn join_style(join: Join, distance: f64) -> LineJoin<f64> {
    match join {
        Join::Round => LineJoin::Round(0.1_f64.min(0.5 / distance.abs().max(0.5))),
        Join::Miter => LineJoin::Miter(0.2),
        Join::Bevel => LineJoin::Bevel,
    }
}

fn contours(lines: &[Polyline]) -> Result<Vec<Vec<[f64; 2]>>, PathOpsError> {
    lines.iter().filter(|l| l.closed && l.points.len() >= 3).map(points).collect()
}

fn points(l: &Polyline) -> Result<Vec<[f64; 2]>, PathOpsError> {
    l.points
        .iter()
        .map(|p| {
            if p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORD_MM && p.y.abs() <= MAX_COORD_MM {
                Ok([p.x, p.y])
            } else {
                Err(PathOpsError::BadCoordinate)
            }
        })
        .collect()
}

fn check_size(sets: &[&Vec<Vec<[f64; 2]>>]) -> Result<(), PathOpsError> {
    let n: usize = sets.iter().flat_map(|s| s.iter()).map(Vec::len).sum();
    if n > MAX_POINTS { Err(PathOpsError::TooLarge) } else { Ok(()) }
}

fn to_polylines(shapes: Vec<Vec<Vec<[f64; 2]>>>) -> Vec<Polyline> {
    shapes
        .into_iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|c| Polyline::new(c.into_iter().map(|p| Point::new(p[0], p[1])).collect(), true))
        .collect()
}

#[cfg(test)]
mod tests;
