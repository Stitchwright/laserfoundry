//! Time model for a planned job: how long each operation takes and where the head is at a given
//! moment. There is no acceleration model, so these are optimistic estimates.

use crate::{Job, Move, Op};
use laserfoundry_doc::LayerId;
use laserfoundry_geom::Point;

/// Travel speed (mm/min) used when the machine's rapid speed is not a usable number.
const DEFAULT_RAPID: f64 = 6000.0;

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

    /// Like [`new`](Timeline::new), with the machine's acceleration in mm/s² (0 = ignore it).
    pub fn with_acceleration(job: &Job, rapid_speed: f64, acceleration: f64) -> Timeline {
        let mut t = Timeline::default();
        // The head's real starting position is unknown; the job is taken to begin at the origin.
        let mut at = Point::ORIGIN;
        for op in &job.ops {
            let (pass_seconds, cut_mm, travel_mm) = one_pass(op, &mut at, rapid_speed, acceleration);
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

fn one_pass(op: &Op, at: &mut Point, rapid_speed: f64, acceleration: f64) -> (f64, f64, f64) {
    let (mut seconds, mut cut, mut travel) = (0.0, 0.0, 0.0);
    for m in &op.moves {
        let d = at.distance(m.target());
        seconds += move_seconds(*at, m, op.speed, rapid_speed, acceleration);
        match m {
            Move::Travel(_) => travel += d,
            Move::Cut { .. } => cut += d,
        }
        *at = m.target();
    }
    (seconds, cut, travel)
}
