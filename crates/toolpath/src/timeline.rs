//! Time model for a planned job: how long each operation takes and where the head is at a given
//! moment. Without an acceleration these are optimistic estimates; with one, moves are timed as a
//! motion planner would run them, carrying speed through gentle corners (see [`Motion`]).

use crate::{Job, Move, Op};
use laserfoundry_doc::LayerId;
use laserfoundry_geom::Point;

/// Travel speed (mm/min) used when the machine's rapid speed is not a usable number.
const DEFAULT_RAPID: f64 = 6000.0;

/// Moves shorter than this (mm) take no time and do not count as corners.
const MIN_LENGTH: f64 = 1e-9;

/// How the controller speeds up and corners, for time estimates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Motion {
    /// mm/s² (GRBL `$120`/`$121`); 0 or a bad number means speed changes are instant.
    pub acceleration: f64,
    /// mm (GRBL `$11`): how far from a corner the path may be cut, which sets how fast the head may take it. 0 or a
    /// bad number means every move starts and ends at rest, which is pessimistic for long smooth curves.
    pub junction_deviation: f64,
}

/// One operation's place on the timeline. All passes of an operation take `pass_seconds` each.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpTime {
    pub layer: LayerId,
    /// Seconds from the start of the job to the first pass.
    pub start: f64,
    pub pass_seconds: f64,
    pub passes: u32,
    /// Distances for one pass.
    pub cut_mm: f64,
    pub travel_mm: f64,
}

/// Time spent on one layer over the whole job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerTime {
    pub layer: LayerId,
    pub seconds: f64,
    pub cut_mm: f64,
    pub travel_mm: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Timeline {
    pub ops: Vec<OpTime>,
    pub total: f64,
}

/// Where the job is at a moment in time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moment {
    pub op: usize,
    /// Zero-based pass being run.
    pub pass: u32,
    /// Seconds into that pass.
    pub into_pass: f64,
}

/// Seconds the head takes to reach `m` from `from`. With an `acceleration` (mm/s²) each move is
/// timed as starting and ending at rest (speeding up and slowing down at that rate), which is
/// pessimistic for long smooth curves but realistic for short segments; 0 means instant speed changes.
pub fn move_seconds(from: Point, m: &Move, speed: f64, rapid_speed: f64, acceleration: f64) -> f64 {
    let d = from.distance(m.target());
    let rapid = if rapid_speed.is_finite() && rapid_speed > 0.0 { rapid_speed } else { DEFAULT_RAPID };
    let feed = match m {
        Move::Travel(_) => rapid,
        Move::Cut { .. } => speed.max(1.0),
    };
    let v = feed / 60.0; // mm/s
    if !(acceleration.is_finite() && acceleration > 0.0) {
        return d / v;
    }
    // Trapezoid: reach `v` and hold it, or (on short moves) never get there.
    let ramp = v * v / acceleration; // distance to speed up and slow down together
    if d >= ramp { d / v + v / acceleration } else { 2.0 * (d / acceleration).sqrt() }
}

impl Timeline {
    pub fn new(job: &Job, rapid_speed: f64) -> Timeline {
        Timeline::with_acceleration(job, rapid_speed, 0.0)
    }

    /// Like [`new`](Timeline::new), with the machine's acceleration in mm/s² (0 = ignore it). Every move is timed
    /// from rest to rest; see [`with_motion`](Timeline::with_motion) for cornering.
    pub fn with_acceleration(job: &Job, rapid_speed: f64, acceleration: f64) -> Timeline {
        Timeline::with_motion(job, rapid_speed, Motion { acceleration, junction_deviation: 0.0 })
    }

    /// Like [`new`](Timeline::new), timing each operation as the controller's planner would run it.
    pub fn with_motion(job: &Job, rapid_speed: f64, motion: Motion) -> Timeline {
        let mut t = Timeline::default();
        // The head's real starting position is unknown; the job is taken to begin at the origin.
        let mut at = Point::ORIGIN;
        for op in &job.ops {
            let (pass_seconds, cut_mm, travel_mm) = one_pass(op, &mut at, rapid_speed, motion);
            let passes = op.passes.max(1);
            t.ops.push(OpTime { layer: op.layer, start: t.total, pass_seconds, passes, cut_mm, travel_mm });
            t.total += pass_seconds * f64::from(passes);
        }
        t
    }

    /// Per-layer totals, in the order layers first appear in the job.
    pub fn layers(&self) -> Vec<LayerTime> {
        let mut out: Vec<LayerTime> = Vec::new();
        for op in &self.ops {
            let n = f64::from(op.passes);
            let add = LayerTime { layer: op.layer, seconds: op.pass_seconds * n, cut_mm: op.cut_mm * n, travel_mm: op.travel_mm * n };
            match out.iter_mut().find(|l| l.layer == op.layer) {
                Some(l) => {
                    l.seconds += add.seconds;
                    l.cut_mm += add.cut_mm;
                    l.travel_mm += add.travel_mm;
                }
                None => out.push(add),
            }
        }
        out
    }

    pub fn cut_mm(&self) -> f64 {
        self.ops.iter().map(|o| o.cut_mm * f64::from(o.passes)).sum()
    }

    pub fn travel_mm(&self) -> f64 {
        self.ops.iter().map(|o| o.travel_mm * f64::from(o.passes)).sum()
    }

    /// The operation, pass and offset running at time `t`. Times past the end land on the end of
    /// the last pass; an empty job has no moment.
    pub fn at(&self, t: f64) -> Option<Moment> {
        let t = if t.is_finite() { t.clamp(0.0, self.total) } else { 0.0 };
        let (op, ot) = self.ops.iter().enumerate().rev().find(|(_, o)| o.start <= t).or_else(|| self.ops.first().map(|o| (0, o)))?;
        let local = t - ot.start;
        if ot.pass_seconds <= 0.0 {
            return Some(Moment { op, pass: 0, into_pass: 0.0 });
        }
        let last = ot.passes.saturating_sub(1);
        let pass = ((local / ot.pass_seconds).floor().max(0.0) as u32).min(last);
        let into_pass = (local - f64::from(pass) * ot.pass_seconds).clamp(0.0, ot.pass_seconds);
        Some(Moment { op, pass, into_pass })
    }
}

/// Seconds each move of `op` takes, one pass starting with the head at `from`. The operation starts and ends at
/// rest. With an acceleration and a junction deviation, a move's entry and exit speeds follow from how sharp its
/// corners are and from how far there is to slow down (the planner looks ahead along the whole operation, which is
/// slightly optimistic: a real controller sees only a limited number of moves ahead). Otherwise each move is timed on
/// its own from rest to rest, as [`move_seconds`] does.
///
/// The corner speed follows the junction-deviation model that GRBL's cornering notes describe
/// (https://onehossshay.wordpress.com/2011/09/24/improving_grbl_cornering_algorithm/): a circle of radius
/// `R = δ·sin(θ/2) / (1 − sin(θ/2))` touches both segments (δ the deviation, θ the angle between them, 180° for a
/// straight line), and the head may take the corner at `sqrt(a·R)`, never above either move's own speed. `$11` is
/// described at https://github.com/gnea/grbl/wiki/Grbl-v1.1-Configuration.
pub fn move_times(op: &Op, from: Point, rapid_speed: f64, motion: Motion) -> Vec<f64> {
    let rest_to_rest = || {
        let mut at = from;
        op.moves
            .iter()
            .map(|m| {
                let seconds = move_seconds(at, m, op.speed, rapid_speed, motion.acceleration);
                at = m.target();
                seconds
            })
            .collect::<Vec<f64>>()
    };
    let a = motion.acceleration;
    let corners = a.is_finite() && a > 0.0 && motion.junction_deviation.is_finite() && motion.junction_deviation > 0.0;
    if !corners {
        return rest_to_rest();
    }

    // The moves that go anywhere, with their direction and their own speed in mm/s.
    let rapid = if rapid_speed.is_finite() && rapid_speed > 0.0 { rapid_speed } else { DEFAULT_RAPID };
    let mut blocks: Vec<Block> = Vec::with_capacity(op.moves.len());
    let mut at = from;
    for (index, m) in op.moves.iter().enumerate() {
        let to = m.target();
        let length = at.distance(to);
        if !length.is_finite() || !at.x.is_finite() || !at.y.is_finite() {
            return rest_to_rest();
        }
        if length > MIN_LENGTH {
            let feed = match m {
                Move::Travel(_) => rapid,
                Move::Cut { .. } => op.speed.max(1.0),
            } / 60.0;
            blocks.push(Block { index, length, dir: ((to.x - at.x) / length, (to.y - at.y) / length), feed });
        }
        at = to;
    }

    // speed[k] is the head's speed entering block k; speed[blocks.len()] is the speed leaving the last one.
    let n = blocks.len();
    let mut speed = vec![0.0_f64; n + 1];
    for k in 1..n {
        let (before, after) = (&blocks[k - 1], &blocks[k]);
        speed[k] = before.feed.min(after.feed).min(corner_speed(before.dir, after.dir, a, motion.junction_deviation));
    }
    for k in (0..n).rev() {
        speed[k] = speed[k].min((speed[k + 1] * speed[k + 1] + 2.0 * a * blocks[k].length).sqrt());
    }
    for k in 0..n {
        speed[k + 1] = speed[k + 1].min((speed[k] * speed[k] + 2.0 * a * blocks[k].length).sqrt());
    }

    let mut seconds = vec![0.0; op.moves.len()];
    for (k, block) in blocks.iter().enumerate() {
        seconds[block.index] = block_seconds(block.length, speed[k], speed[k + 1], block.feed, a);
    }
    seconds
}

/// A move that goes somewhere, for the planner.
struct Block {
    /// Which move of the operation it is.
    index: usize,
    length: f64,
    /// Unit vector of travel.
    dir: (f64, f64),
    /// The speed it is run at, mm/s.
    feed: f64,
}

/// The fastest the head may take the corner between two directions of travel, mm/s (infinite on a straight line).
pub(crate) fn corner_speed(before: (f64, f64), after: (f64, f64), acceleration: f64, deviation: f64) -> f64 {
    let straight = (before.0 * after.0 + before.1 * after.1).clamp(-1.0, 1.0); // 1 straight on, -1 straight back
    let half = ((1.0 + straight) / 2.0).sqrt(); // sin(θ/2), θ the angle between the two segments
    if half >= 1.0 - 1e-12 {
        return f64::INFINITY;
    }
    (acceleration * deviation * half / (1.0 - half)).sqrt()
}

/// Seconds to cover `length` mm, entering at `v0` and leaving at `v1` mm/s with at most `top` mm/s in between.
fn block_seconds(length: f64, v0: f64, v1: f64, top: f64, a: f64) -> f64 {
    let (v0, v1) = (v0.min(top), v1.min(top));
    let (up, down) = ((top * top - v0 * v0) / (2.0 * a), (top * top - v1 * v1) / (2.0 * a));
    if up + down <= length {
        return (top - v0) / a + (top - v1) / a + (length - up - down) / top;
    }
    // Too short to reach the top speed: speed up, then slow down, meeting at the highest speed that fits.
    let peak = ((2.0 * a * length + v0 * v0 + v1 * v1) / 2.0).sqrt().max(v0).max(v1);
    (peak - v0) / a + (peak - v1) / a
}

fn one_pass(op: &Op, at: &mut Point, rapid_speed: f64, motion: Motion) -> (f64, f64, f64) {
    let times = move_times(op, *at, rapid_speed, motion);
    let (mut seconds, mut cut, mut travel) = (0.0, 0.0, 0.0);
    for (m, t) in op.moves.iter().zip(times) {
        let d = at.distance(m.target());
        seconds += t;
        match m {
            Move::Travel(_) => travel += d,
            Move::Cut { .. } => cut += d,
        }
        *at = m.target();
    }
    (seconds, cut, travel)
}
