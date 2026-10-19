//! Toolpath planning: turns a [`Document`] into a [`Job`] of machine-independent moves.
//!
//! Layering: knows geometry and the document, nothing about G-code dialects or devices.

mod fill;
mod order;
mod overlap;
mod palette;
mod perforate;
mod raster;
mod tabs;
mod timeline;
mod trace;

pub use fill::fill_segments;
pub use order::{order_polylines, remove_duplicates};
pub use overlap::remove_overlaps;
pub use palette::{BACKGROUND, MAX_COLORS, Quantized, quantize};
pub use raster::{Grid, dither, sample_grid};
pub use timeline::{LayerTime, Moment, OpTime, Timeline, move_seconds};
pub use trace::{MAX_TRACE_PIXELS, trace};

use laserfoundry_doc::{Document, Layer, LayerId, LayerMode, Shape, ShapeKind};
use laserfoundry_geom::{Point, Polyline, Rect, bounds_of_polylines};
use thiserror::Error;

/// Cap on raster cells per image, so a hostile document cannot exhaust memory.
pub const MAX_RASTER_CELLS: usize = 25_000_000;
/// Duplicate removal is skipped for layers with more outlines than this (it is memory-hungry).
const MAX_DEDUPLICATED: usize = 200_000;
/// Largest Z offset or per-pass step accepted, in mm.
pub const MAX_Z_MM: f64 = 200.0;
/// Largest overscan and ramp length accepted, in mm; ramps are built from steps of this size.
pub const MAX_OVERSCAN_MM: f64 = 50.0;
pub const MAX_RAMP_MM: f64 = 1000.0;
const RAMP_STEP_MM: f64 = 0.5;
/// Cap on total moves in a job.
pub const MAX_MOVES: usize = 20_000_000;

/// A planned job, in layer order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Job {
    pub ops: Vec<Op>,
}

/// All moves for one layer (or one image) and the settings they run with.
#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    pub layer: LayerId,
    pub kind: OpKind,
    /// mm/min
    pub speed: f64,
    pub passes: u32,
    pub air_assist: bool,
    /// Z height of the first pass and the change for each later pass (mm); both 0 = leave Z alone.
    pub z: f64,
    pub z_per_pass: f64,
    pub moves: Vec<Move>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Line,
    Fill,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Move {
    /// Rapid, laser off.
    Travel(Point),
    /// Feed move; `power` is a percentage of full power (0 = laser off, used for overscan and gaps).
    Cut { to: Point, power: f64 },
}

impl Move {
    pub fn target(&self) -> Point {
        match self {
            Move::Travel(p) => *p,
            Move::Cut { to, .. } => *to,
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum PlanError {
    #[error("layer {name}: {what} must be a positive finite number")]
    BadLayerSetting { name: String, what: &'static str },
    #[error("image raster would need {0} cells (limit {MAX_RASTER_CELLS}); increase the line interval")]
    RasterTooLarge(usize),
    #[error("job has more than {MAX_MOVES} moves")]
    TooManyMoves,
    #[error("layer {name}: kerf offset failed: {reason}")]
    Kerf { name: String, reason: String },
}

/// Plans every output-enabled layer. Layers are cut in id order; within a layer fills run before lines.
pub fn plan(doc: &Document) -> Result<Job, PlanError> {
    let mut job = Job::default();
    let mut total = 0usize;
    let mut at = match doc.optimize.start {
        Some([x, y]) if x.is_finite() && y.is_finite() => Point::new(x, y),
        _ => Point::ORIGIN,
    };
    for layer in doc.layers.iter().filter(|l| l.output) {
        validate(layer)?;
        let all: Vec<&Shape> = doc.shapes.iter().filter(|s| s.layer == layer.id).collect();
        // Lower cut priorities are finished before higher ones (all shapes default to 0).
        let mut tiers: Vec<i32> = all.iter().map(|s| s.order).collect();
        tiers.sort_unstable();
        tiers.dedup();
        for tier in tiers {
            let shapes: Vec<&Shape> = all.iter().copied().filter(|s| s.order == tier).collect();
            // Shapes with their own cut settings are planned separately, in order of first appearance;
            // everything else uses the layer as it is.
            let mut groups: Vec<(Option<laserfoundry_doc::CutOverride>, Vec<&Shape>)> = Vec::new();
            for shape in shapes {
                let key = shape.cut.filter(|c| !c.is_empty());
                match groups.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, v)) => v.push(shape),
                    None => groups.push((key, vec![shape])),
                }
            }
            groups.sort_by_key(|(k, _)| k.is_some()); // stable: plain shapes first
            for (key, group) in groups {
                let effective = key.map(|c| c.apply(layer));
                let layer_for_group = match &effective {
                    Some(l) => {
                        validate(l)?;
                        l
                    }
                    None => layer,
                };
                // With "by group", each group of shapes is finished before the next one starts.
                let buckets: Vec<Vec<&Shape>> = if doc.optimize.by_group { split_by_group(&group) } else { vec![group] };
                for bucket in buckets {
                    let mut ops = plan_layer(doc, layer_for_group, &bucket, at)?;
                    for op in &mut ops {
                        total = total.saturating_add(op.moves.len());
                        if let Some(last) = op.moves.last() {
                            at = last.target();
                        }
                    }
                    if total > MAX_MOVES {
                        return Err(PlanError::TooManyMoves);
                    }
                    job.ops.append(&mut ops);
                }
            }
        }
    }
    Ok(job)
}

/// Shapes bucketed by group, buckets in order of first appearance; ungrouped shapes share one bucket.
fn split_by_group<'a>(shapes: &[&'a Shape]) -> Vec<Vec<&'a Shape>> {
    let mut buckets: Vec<(Option<laserfoundry_doc::GroupId>, Vec<&'a Shape>)> = Vec::new();
    for shape in shapes {
        match buckets.iter_mut().find(|(g, _)| *g == shape.group) {
            Some((_, v)) => v.push(shape),
            None => buckets.push((shape.group, vec![shape])),
        }
    }
    buckets.into_iter().map(|(_, v)| v).collect()
}

/// Appends the moves for one outline: an optional lead-in, the outline itself (with tabs on
/// material outlines when the layer asks for them), and an optional lead-out.
fn emit_outline(moves: &mut Vec<Move>, doc: &Document, layer: &Layer, pl: &Polyline, depth: usize, power: f64) {
    let Some(first) = pl.points.first().copied() else { return };
    let perforated =
        if layer.perf_cut > 0.0 && layer.perf_gap > 0.0 { perforate::perforate(&pl.points, pl.closed, layer.perf_cut, layer.perf_gap) } else { None };
    if !pl.closed {
        moves.push(Move::Travel(first));
        match &perforated {
            Some(pieces) => emit_dashes(moves, pieces, power),
            None => cut_with_ramp(moves, &pl.points, false, layer, power),
        }
        return;
    }
    // Leads need a start in the middle of an edge (see `restart_mid_edge`).
    let restarted;
    let (pl, first) = if layer.lead_in > 0.0 || layer.lead_out > 0.0 {
        restarted = tabs::restart_mid_edge(pl);
        (&restarted, restarted.points.first().copied().unwrap_or(first))
    } else {
        (pl, first)
    };
    let lead = |len: f64| tabs::lead_point(pl, depth, len).map(|p| keep_in_bed(doc, first, p));
    match lead(layer.lead_in) {
        Some(entry) => {
            moves.push(Move::Travel(entry));
            moves.push(Move::Cut { to: first, power });
        }
        None => moves.push(Move::Travel(first)),
    }
    // Perforation replaces tabs: a dashed line is already held together by its gaps.
    let pieces = if layer.tab_count > 0 && depth.is_multiple_of(2) { tabs::split_for_tabs(pl, layer.tab_count, layer.tab_width) } else { None };
    if layer.perf_cut > 0.0
        && layer.perf_gap > 0.0
        && let Some(dashes) = perforate::perforate(&pl.points, true, layer.perf_cut, layer.perf_gap)
    {
        emit_dashes(moves, &dashes, power);
        if let Some(exit) = lead(layer.lead_out) {
            moves.push(Move::Cut { to: exit, power });
        }
        return;
    }
    match pieces {
        Some(pieces) => {
            for piece in &pieces {
                let p = if piece.on { power } else { 0.0 };
                moves.extend(piece.points.iter().skip(1).map(|pt| Move::Cut { to: *pt, power: p }));
            }
        }
        None => cut_with_ramp(moves, &pl.points, true, layer, power),
    }
    if let Some(exit) = lead(layer.lead_out) {
        moves.push(Move::Cut { to: exit, power });
    }
}

/// Cuts along `points` (and back to the start for closed outlines), raising power from the layer's
/// minimum to `power` over its ramp length when it has one.
fn cut_with_ramp(moves: &mut Vec<Move>, points: &[Point], closed: bool, layer: &Layer, power: f64) {
    let ramp = if layer.ramp_length.is_finite() { layer.ramp_length.clamp(0.0, MAX_RAMP_MM) } else { 0.0 };
    let low = layer.min_power.clamp(0.0, power);
    let Some(first) = points.first().copied() else { return };
    let mut path: Vec<Point> = points.iter().skip(1).copied().collect();
    if closed {
        path.push(first);
    }
    if ramp <= 0.0 || low >= power {
        moves.extend(path.into_iter().map(|p| Move::Cut { to: p, power }));
        return;
    }
    let mut at = first;
    let mut done = 0.0;
    for to in path {
        let seg = at.distance(to);
        if seg <= 0.0 {
            continue;
        }
        let mut pos = 0.0;
        while pos < seg {
            if done >= ramp {
                moves.push(Move::Cut { to, power });
                break;
            }
            let step = (seg - pos).min(RAMP_STEP_MM).min(ramp - done).max(1e-6);
            let mid = (done + step / 2.0) / ramp;
            let end = at.lerp(to, ((pos + step) / seg).min(1.0));
            moves.push(Move::Cut { to: end, power: low + (power - low) * mid.clamp(0.0, 1.0) });
            pos += step;
            done += step;
        }
        at = to;
    }
}

/// Cuts the "on" pieces and moves over the gaps with the laser off (rapid moves).
fn emit_dashes(moves: &mut Vec<Move>, pieces: &[tabs::Piece], power: f64) {
    for piece in pieces {
        for p in piece.points.iter().skip(1) {
            moves.push(if piece.on { Move::Cut { to: *p, power } } else { Move::Travel(*p) });
        }
    }
}

/// With absolute placement the bed edges are known: shorten a lead that would leave the bed.
fn keep_in_bed(doc: &Document, from: Point, to: Point) -> Point {
    if doc.job_origin.start_from != laserfoundry_doc::StartFrom::Absolute {
        return to;
    }
    let (w, h) = (doc.machine.bed_width, doc.machine.bed_height);
    let mut t: f64 = 1.0;
    for (a, b, hi) in [(from.x, to.x, w), (from.y, to.y, h)] {
        if b < 0.0 && a != b {
            t = t.min(a / (a - b));
        } else if b > hi && a != b {
            t = t.min((hi - a) / (b - a));
        }
    }
    from.lerp(to, t.clamp(0.0, 1.0))
}

/// Offsets closed outlines by the layer's kerf compensation; open paths are left as they are.
fn apply_kerf(layer: &Layer, lines: &[Polyline]) -> Result<Vec<Polyline>, PlanError> {
    if layer.kerf_offset == 0.0 {
        return Ok(lines.to_vec());
    }
    let (closed, open): (Vec<&Polyline>, Vec<&Polyline>) = lines.iter().partition(|l| l.closed);
    let closed: Vec<Polyline> = closed.into_iter().cloned().collect();
    let mut out = laserfoundry_pathops::offset(&closed, layer.kerf_offset, laserfoundry_pathops::Join::Round)
        .map_err(|e| PlanError::Kerf { name: layer.name.clone(), reason: e.to_string() })?;
    out.extend(open.into_iter().cloned());
    Ok(out)
}

/// Bounds of everything that will be output (the "frame"), in workspace mm.
pub fn job_bounds(doc: &Document) -> Option<Rect> {
    let lines: Vec<Polyline> = doc.shapes.iter().filter(|s| doc.layer(s.layer).is_some_and(|l| l.output)).flat_map(Shape::world_polylines).collect();
    bounds_of_polylines(&lines)
}

/// Rough time estimate (no acceleration model).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Estimate {
    pub seconds: f64,
    pub cut_mm: f64,
    pub travel_mm: f64,
}

pub fn estimate(job: &Job, rapid_speed: f64) -> Estimate {
    let t = Timeline::new(job, rapid_speed);
    Estimate { seconds: t.total, cut_mm: t.cut_mm(), travel_mm: t.travel_mm() }
}

fn validate(layer: &Layer) -> Result<(), PlanError> {
    let bad = |what| PlanError::BadLayerSetting { name: layer.name.clone(), what };
    if !(layer.speed.is_finite() && layer.speed > 0.0) {
        return Err(bad("speed"));
    }
    if !(layer.interval.is_finite() && layer.interval > 0.0) {
        return Err(bad("interval"));
    }
    if !(layer.max_power.is_finite() && layer.min_power.is_finite()) {
        return Err(bad("power"));
    }
    if !(layer.kerf_offset.is_finite() && layer.kerf_offset.abs() <= 50.0) {
        return Err(bad("kerf offset"));
    }
    if let Some(o) = layer.overscan
        && !(o.is_finite() && (0.0..=MAX_OVERSCAN_MM).contains(&o))
    {
        return Err(bad("overscan"));
    }
    if !(layer.ramp_length.is_finite() && (0.0..=MAX_RAMP_MM).contains(&layer.ramp_length)) {
        return Err(bad("ramp length"));
    }
    for (value, what) in [(layer.z_offset, "Z offset"), (layer.z_per_pass, "Z step per pass")] {
        if !(value.is_finite() && value.abs() <= MAX_Z_MM) {
            return Err(bad(what));
        }
    }
    for (value, what) in [(layer.perf_cut, "perforation cut length"), (layer.perf_gap, "perforation gap")] {
        if !(value.is_finite() && (0.0..=1000.0).contains(&value)) {
            return Err(bad(what));
        }
    }
    for (value, what) in [(layer.tab_width, "tab width"), (layer.lead_in, "lead-in"), (layer.lead_out, "lead-out")] {
        if !(value.is_finite() && (0.0..=1000.0).contains(&value)) {
            return Err(bad(what));
        }
    }
    Ok(())
}

/// The run-up and run-out length for a layer: its own value, else (with `overscan_by_speed` and a
/// known acceleration) the distance needed to reach its speed, else `unset`.
pub(crate) fn overscan_for(doc: &Document, layer: &Layer, unset: f64) -> f64 {
    let m = &doc.machine;
    let wanted = match layer.overscan {
        Some(o) => o,
        None if m.overscan_by_speed && m.acceleration.is_finite() && m.acceleration > 0.0 && layer.speed.is_finite() && layer.speed > 0.0 => {
            let v = layer.speed / 60.0;
            v * v / (2.0 * m.acceleration)
        }
        None => unset,
    };
    if wanted.is_finite() { wanted.clamp(0.0, MAX_OVERSCAN_MM) } else { 0.0 }
}

fn plan_layer(doc: &Document, layer: &Layer, shapes: &[&Shape], start: Point) -> Result<Vec<Op>, PlanError> {
    let mut ops = Vec::new();
    let mut vector: Vec<Polyline> = shapes.iter().filter(|s| s.vector_path().is_some()).flat_map(|s| s.world_polylines()).collect();
    if doc.optimize.remove_duplicates && vector.len() <= MAX_DEDUPLICATED {
        vector = remove_duplicates(vector);
    }
    let mut outlines = apply_kerf(layer, &vector)?;
    if doc.optimize.remove_duplicates && outlines.len() <= MAX_DEDUPLICATED {
        outlines = remove_overlaps(outlines);
    }
    let power = layer.max_power.clamp(0.0, 100.0);
    let passes = layer.passes.clamp(1, 1000);
    let mk = |kind, moves| Op {
        layer: layer.id,
        kind,
        speed: layer.speed,
        passes,
        air_assist: layer.air_assist,
        z: layer.z_offset,
        z_per_pass: layer.z_per_pass,
        moves,
    };

    if matches!(layer.mode, LayerMode::Fill | LayerMode::FillAndLine) {
        let angles: &[f64] = if layer.crosshatch { &[0.0, 90.0] } else { &[0.0] };
        for extra in angles {
            let segs = fill_segments(&vector, layer.interval, layer.scan_angle + extra, layer.bidirectional);
            if segs.is_empty() {
                continue;
            }
            let over = overscan_for(doc, layer, 0.0);
            let mut moves = Vec::with_capacity(segs.len() * 2);
            for (a, b) in segs {
                let len = a.distance(b);
                if over > 0.0 && len > 0.0 {
                    // Run up to speed before the line and slow down after it, laser off.
                    let dir = (b - a) / len;
                    moves.push(Move::Travel(keep_in_bed(doc, a, a - dir * over)));
                    moves.push(Move::Cut { to: a, power: 0.0 });
                    moves.push(Move::Cut { to: b, power });
                    moves.push(Move::Cut { to: keep_in_bed(doc, b, b + dir * over), power: 0.0 });
                } else {
                    moves.push(Move::Travel(a));
                    moves.push(Move::Cut { to: b, power });
                }
            }
            ops.push(mk(OpKind::Fill, moves));
        }
    }
    if matches!(layer.mode, LayerMode::Line | LayerMode::FillAndLine) && !outlines.is_empty() {
        let mut moves = Vec::new();
        let ordered = order_polylines(outlines, start, &doc.optimize);
        // Nesting is quadratic; very large jobs skip tabs and leads rather than stall.
        let depths = if ordered.len() <= 4000 { tabs::nesting_depths(&ordered) } else { vec![0; ordered.len()] };
        for (pl, depth) in ordered.iter().zip(depths) {
            emit_outline(&mut moves, doc, layer, pl, depth, power);
        }
        ops.push(mk(OpKind::Line, moves));
    }
    for shape in shapes {
        if let ShapeKind::Image { .. } = shape.kind {
            let moves = raster::plan_image(doc, layer, shape)?;
            if !moves.is_empty() {
                ops.push(mk(OpKind::Image, moves));
            }
        }
    }
    Ok(ops)
}

#[cfg(test)]
mod tests;
