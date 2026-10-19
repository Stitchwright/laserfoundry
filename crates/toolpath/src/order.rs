use std::collections::HashSet;

use laserfoundry_doc::Optimize;
use laserfoundry_geom::{Point, Polyline};

/// Coordinates closer than this (mm) count as the same point when looking for duplicates.
const SAME_POINT_MM: f64 = 1e-4;

/// Orders polylines for cutting. With `inner_first`, shapes nested inside others come first (so
/// parts don't drop out of the sheet before their holes are cut); with `reduce_travel`, the next
/// outline is the one nearest the head from `start`, and closed outlines are rotated to begin at
/// their vertex nearest it. Without it the drawing order is kept (apart from the inner-first sort).
pub fn order_polylines(lines: Vec<Polyline>, start: Point, opts: &Optimize) -> Vec<Polyline> {
    const MAX_FOR_FULL_ORDERING: usize = 4000;
    if lines.len() > MAX_FOR_FULL_ORDERING || !(opts.inner_first || opts.reduce_travel) {
        return lines;
    }
    let depths: Vec<usize> = (0..lines.len()).map(|i| if opts.inner_first { depth(&lines, i) } else { 0 }).collect();
    if !opts.reduce_travel {
        let mut keyed: Vec<(usize, Polyline)> = depths.into_iter().zip(lines).collect();
        keyed.sort_by_key(|(d, _)| std::cmp::Reverse(*d)); // stable: ties keep the drawing order
        return keyed.into_iter().map(|(_, l)| l).collect();
    }
    let mut remaining: Vec<(usize, Polyline)> = depths.into_iter().zip(lines).collect();
    let mut out = Vec::with_capacity(remaining.len());
    let mut at = start;
    while !remaining.is_empty() {
        let max_depth = remaining.iter().map(|(d, _)| *d).max().unwrap_or(0);
        let mut best: Option<(usize, bool, f64)> = None;
        for (i, (d, pl)) in remaining.iter().enumerate() {
            if *d != max_depth {
                continue;
            }
            let (Some(first), Some(last)) = (pl.points.first(), pl.points.last()) else { continue };
            let nearest_vertex =
                if pl.closed { pl.points.iter().map(|p| at.distance(*p)).fold(f64::INFINITY, f64::min) } else { at.distance(*first) };
            let rev_dist = if pl.closed { f64::INFINITY } else { at.distance(*last) };
            let (dist, rev) = if rev_dist < nearest_vertex { (rev_dist, true) } else { (nearest_vertex, false) };
            if best.is_none_or(|(_, _, bd)| dist < bd) {
                best = Some((i, rev, dist));
            }
        }
        let Some((i, rev, _)) = best else { break };
        let (_, mut pl) = remaining.swap_remove(i);
        if rev {
            pl.points.reverse();
        } else if pl.closed {
            rotate_to_nearest(&mut pl, at);
        }
        if let Some(end) = if pl.closed { pl.points.first() } else { pl.points.last() } {
            at = *end;
        }
        out.push(pl);
    }
    out
}

fn depth(lines: &[Polyline], i: usize) -> usize {
    let Some(p) = lines.get(i).and_then(|l| l.points.first()) else { return 0 };
    lines.iter().enumerate().filter(|(j, other)| *j != i && other.contains(*p)).count()
}

fn rotate_to_nearest(pl: &mut Polyline, at: Point) {
    let idx = pl.points.iter().enumerate().min_by(|a, b| at.distance(*a.1).total_cmp(&at.distance(*b.1))).map(|(i, _)| i).unwrap_or(0);
    pl.points.rotate_left(idx);
}

/// Removes outlines that coincide with an earlier one: same points, in either direction, and for
/// closed outlines from any starting vertex. Lines with non-finite coordinates are kept as they are.
pub fn remove_duplicates(lines: Vec<Polyline>) -> Vec<Polyline> {
    let mut seen: HashSet<(bool, Vec<(i64, i64)>)> = HashSet::new();
    lines
        .into_iter()
        .filter(|l| match canonical(l) {
            Some(key) => seen.insert(key),
            None => true,
        })
        .collect()
}

fn canonical(l: &Polyline) -> Option<(bool, Vec<(i64, i64)>)> {
    let mut pts: Vec<(i64, i64)> = Vec::with_capacity(l.points.len());
    for p in &l.points {
        if !(p.x.is_finite() && p.y.is_finite() && p.x.abs() < 1e9 && p.y.abs() < 1e9) {
            return None;
        }
        pts.push(((p.x / SAME_POINT_MM).round() as i64, (p.y / SAME_POINT_MM).round() as i64));
    }
    if l.closed {
        // A closing point that repeats the first one does not change the shape.
        if pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        let smallest = |v: &[(i64, i64)]| v.iter().enumerate().min_by_key(|(_, p)| **p).map_or(0, |(i, _)| i);
        let mut fwd = pts.clone();
        let k = smallest(&fwd);
        fwd.rotate_left(k);
        let mut rev = pts;
        rev.reverse();
        let k = smallest(&rev);
        rev.rotate_left(k);
        return Some((true, fwd.min(rev)));
    }
    let mut rev = pts.clone();
    rev.reverse();
    Some((false, pts.min(rev)))
}
