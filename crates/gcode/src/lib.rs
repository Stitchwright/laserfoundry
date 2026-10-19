//! GRBL-flavoured G-code generation from a planned [`Job`].
//!
//! Workspace coordinates are y-down from the bed's top-left; the machine profile decides how they
//! map onto machine coordinates.

mod resume;

pub use resume::{ResumeError, resume_from};

use std::fmt::Write;

use laserfoundry_doc::{Dialect, Machine, Origin};
use laserfoundry_geom::{Point, Rect};
use laserfoundry_toolpath::{Job, Move};
use thiserror::Error;

/// Largest feed rate written, mm/min.
pub const MAX_FEED: f64 = 1_000_000.0;
/// Slack (mm) allowed outside the bed before a job is refused.
const BED_EPSILON: f64 = 1e-3;

#[derive(Debug, Error, PartialEq)]
pub enum GcodeError {
    #[error("point ({x:.2}, {y:.2}) mm is outside the {w:.0}×{h:.0} mm work area")]
    OutOfBed { x: f64, y: f64, w: f64, h: f64 },
    #[error("job contains a non-finite coordinate")]
    NonFinite,
    #[error("feed rate {0} is not a finite positive number")]
    BadFeed(f64),
    #[error("a move lies far outside the job's declared extent")]
    OutsideExtent,
    #[error("machine {0} must be a positive finite number")]
    BadMachine(&'static str),
    #[error("the job is {w:.1}×{h:.1} mm, larger than the {bw:.0}×{bh:.0} mm work area")]
    JobLargerThanBed { w: f64, h: f64, bw: f64, bh: f64 },
}

/// Where machine coordinates come from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// Workspace coordinates are machine coordinates (the machine is homed / zeroed at its bed corner).
    Absolute,
    /// Coordinates are relative to a job anchor that sits at the machine's work-zero. The bed cannot
    /// be checked point by point (the zero is wherever the user put it), so the job's extent must
    /// fit within the bed instead.
    Relative {
        /// The job-bounds point (workspace mm) that maps to work-zero.
        anchor: Point,
        /// `(width, height)` of the job bounds, mm.
        extent: (f64, f64),
        /// Emit `G92 X0 Y0` so the head's current position becomes zero ("Current Position").
        set_zero: bool,
    },
}

/// Generates a complete program: preamble, every op × passes, shutdown.
pub fn generate(job: &Job, machine: &Machine, placement: Placement) -> Result<String, GcodeError> {
    check_machine(machine)?;
    let mut w = Writer::new(machine, placement)?;
    w.preamble("LaserFoundry job");
    for op in &job.ops {
        if !(op.speed.is_finite() && op.speed > 0.0 && op.speed <= MAX_FEED) {
            return Err(GcodeError::BadFeed(op.speed));
        }
        if op.air_assist {
            w.line("M8");
        }
        for pass in 0..op.passes.max(1) {
            w.comment(&format!("layer {} pass {}", op.layer, pass + 1));
            w.set_z(op.z + op.z_per_pass * f64::from(pass), op.z != 0.0 || op.z_per_pass != 0.0)?;
            for m in &op.moves {
                match m {
                    Move::Travel(p) => w.travel(*p)?,
                    Move::Cut { to, power } => w.cut(*to, *power, op.speed)?,
                }
            }
        }
        if op.air_assist {
            w.line("M9");
        }
    }
    w.shutdown();
    Ok(w.out)
}

/// A program that traces `bounds` at low power (`power` percent, 0 for a dry run), `loops` times.
pub fn frame(bounds: Rect, machine: &Machine, placement: Placement, speed: f64, power: f64) -> Result<String, GcodeError> {
    check_machine(machine)?;
    if !(speed.is_finite() && speed > 0.0) {
        return Err(GcodeError::BadMachine("frame speed"));
    }
    let mut w = Writer::new(machine, placement)?;
    w.preamble("LaserFoundry frame");
    let corners = [(bounds.x0, bounds.y0), (bounds.x1, bounds.y0), (bounds.x1, bounds.y1), (bounds.x0, bounds.y1), (bounds.x0, bounds.y0)];
    for (i, (x, y)) in corners.iter().enumerate() {
        let p = Point::new(*x, *y);
        if i == 0 {
            w.travel(p)?;
        } else {
            w.cut(p, power, speed)?;
        }
    }
    w.shutdown();
    Ok(w.out)
}

fn check_machine(m: &Machine) -> Result<(), GcodeError> {
    for (name, v) in [("bed width", m.bed_width), ("bed height", m.bed_height)] {
        if !(v.is_finite() && v > 0.0) {
            return Err(GcodeError::BadMachine(name));
        }
    }
    let r = m.rotary;
    if r.enabled && !(r.reference_diameter.is_finite() && r.reference_diameter > 0.0 && r.object_diameter.is_finite() && r.object_diameter > 0.0) {
        return Err(GcodeError::BadMachine("rotary diameters"));
    }
    if m.max_s == 0 {
        return Err(GcodeError::BadMachine("max S value"));
    }
    Ok(())
}

struct Writer<'a> {
    out: String,
    machine: &'a Machine,
    placement: Placement,
    x: Option<f64>,
    y: Option<f64>,
    s: Option<u32>,
    f: Option<f64>,
    /// Current Z when the program has moved it; `None` while Z is untouched.
    z: Option<f64>,
}

impl<'a> Writer<'a> {
    fn new(machine: &'a Machine, placement: Placement) -> Result<Self, GcodeError> {
        if let Placement::Relative { extent: (w, h), anchor, .. } = placement {
            if !(w.is_finite() && h.is_finite() && w >= 0.0 && h >= 0.0 && anchor.x.is_finite() && anchor.y.is_finite()) {
                return Err(GcodeError::NonFinite);
            }
            if w > machine.bed_width + BED_EPSILON || h > machine.bed_height + BED_EPSILON {
                return Err(GcodeError::JobLargerThanBed { w, h, bw: machine.bed_width, bh: machine.bed_height });
            }
        }
        Ok(Writer { out: String::new(), machine, placement, x: None, y: None, s: None, f: None, z: None })
    }

    fn preamble(&mut self, title: &str) {
        self.comment(title);
        self.line("G21");
        self.line("G90");
        if matches!(self.placement, Placement::Relative { set_zero: true, .. }) {
            self.line("G92 X0 Y0");
        }
        match self.machine.dialect {
            Dialect::Grbl => {
                self.line(if self.machine.dynamic_power { "M4 S0" } else { "M3 S0" });
                self.s = Some(0);
            }
            Dialect::Marlin => {
                // Inline mode: power then follows the S word of each G1 and the laser is off for G0.
                self.line("M3 I S0");
                self.s = Some(0);
            }
            // The laser module fires on G1 with the S of that move; no spindle command is needed.
            Dialect::Smoothie => self.s = None,
        }
    }

    fn shutdown(&mut self) {
        self.line("M5");
        self.line("G0 X0 Y0");
    }

    /// Moves Z (with the laser off) when an operation wants a different height; an operation that
    /// does not use Z brings it back to the work zero if an earlier one moved it.
    fn set_z(&mut self, z: f64, wanted: bool) -> Result<(), GcodeError> {
        if !z.is_finite() || z.abs() > laserfoundry_toolpath::MAX_Z_MM {
            return Err(GcodeError::NonFinite);
        }
        let target = if wanted { z } else { 0.0 };
        let current = self.z.unwrap_or(0.0);
        if (target - current).abs() < 1e-9 {
            return Ok(());
        }
        // Feed moves are modal in GRBL, so a plain rapid is fine: the beam is off during G0.
        self.line(&format!("G0 Z{}", num(target)));
        self.z = Some(target);
        Ok(())
    }

    fn travel(&mut self, p: Point) -> Result<(), GcodeError> {
        let (x, y) = self.to_machine(p)?;
        if self.x == Some(x) && self.y == Some(y) {
            return Ok(());
        }
        let mut s = String::from("G0");
        self.push_axes(&mut s, x, y);
        self.line(&s);
        Ok(())
    }

    fn cut(&mut self, p: Point, power: f64, speed: f64) -> Result<(), GcodeError> {
        if !power.is_finite() {
            return Err(GcodeError::NonFinite);
        }
        let (x, y) = self.to_machine(p)?;
        let (sv, text) = self.power_word(power);
        if self.x == Some(x) && self.y == Some(y) && self.s == Some(sv) {
            return Ok(());
        }
        let mut s = String::from("G1");
        self.push_axes(&mut s, x, y);
        if self.s != Some(sv) {
            let _ = write!(s, " S{text}");
            self.s = Some(sv);
        }
        if self.f != Some(speed) {
            let _ = write!(s, " F{}", num(speed));
            self.f = Some(speed);
        }
        self.line(&s);
        Ok(())
    }

    /// The S value for a power percentage: a key for change detection and its text.
    fn power_word(&self, power: f64) -> (u32, String) {
        let fraction = power.clamp(0.0, 100.0) / 100.0;
        match self.machine.dialect {
            // Smoothieware takes 0..1 by default; thousandths is plenty of resolution.
            Dialect::Smoothie => {
                let milli = (fraction * 1000.0).round() as u32;
                (milli, num(f64::from(milli) / 1000.0))
            }
            Dialect::Grbl | Dialect::Marlin => {
                let sv = (fraction * self.machine.max_s as f64).round() as u32;
                (sv, sv.to_string())
            }
        }
    }

    fn push_axes(&mut self, s: &mut String, x: f64, y: f64) {
        if self.x != Some(x) {
            let _ = write!(s, " X{}", num(x));
        }
        if self.y != Some(y) {
            let _ = write!(s, " Y{}", num(y));
        }
        self.x = Some(x);
        self.y = Some(y);
    }

    fn to_machine(&self, p: Point) -> Result<(f64, f64), GcodeError> {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return Err(GcodeError::NonFinite);
        }
        let m = self.machine;
        if let Placement::Relative { anchor, .. } = self.placement {
            let (dx, dy) = (p.x - anchor.x, p.y - anchor.y);
            // Lead-ins and overscan may leave the job's own bounds a little, but never by more than
            // a bed's width: that would mean the declared extent does not describe this job.
            let (ew, eh) = match self.placement {
                Placement::Relative { extent, .. } => extent,
                Placement::Absolute => (0.0, 0.0),
            };
            if dx.abs() > ew + m.bed_width || dy.abs() > eh + m.bed_height {
                return Err(GcodeError::OutsideExtent);
            }
            let y = match m.origin {
                Origin::TopLeft => dy,
                Origin::BottomLeft => -dy,
            };
            return Ok((round3(dx), round3(y * m.rotary.y_scale())));
        }
        if p.x < -BED_EPSILON || p.y < -BED_EPSILON || p.x > m.bed_width + BED_EPSILON || p.y > m.bed_height + BED_EPSILON {
            return Err(GcodeError::OutOfBed { x: p.x, y: p.y, w: m.bed_width, h: m.bed_height });
        }
        let y = match m.origin {
            Origin::TopLeft => p.y,
            Origin::BottomLeft => m.bed_height - p.y,
        };
        Ok((round3(p.x.clamp(0.0, m.bed_width)), round3(y.clamp(0.0, m.bed_height) * m.rotary.y_scale())))
    }

    fn line(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn comment(&mut self, s: &str) {
        let clean: String = s.chars().filter(|c| !matches!(c, '(' | ')' | ';' | '\n' | '\r')).collect();
        self.out.push_str("; ");
        self.out.push_str(&clean);
        self.out.push('\n');
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Formats with up to 3 decimals, no trailing zeros, and never `-0`.
fn num(v: f64) -> String {
    let s = format!("{:.3}", round3(v));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use laserfoundry_doc::{Document, Shape};
    use laserfoundry_geom::rect_path;
    use laserfoundry_toolpath::plan;

    use super::*;

    fn rect_program(origin: Origin) -> String {
        let mut doc = Document::default();
        doc.machine.origin = origin;
        doc.machine.bed_height = 100.0;
        if let Some(l) = doc.layer_mut(0) {
            l.speed = 1200.0;
            l.max_power = 50.0;
        }
        doc.add_shape(Shape::path("r", 0, rect_path(10.0, 10.0, 20.0, 10.0)));
        generate(&plan(&doc).unwrap(), &doc.machine, Placement::Absolute).unwrap()
    }

    mod refusal_tests {
        use super::*;
        use laserfoundry_toolpath::{Op, OpKind};

        fn job(speed: f64, power: f64, to: Point) -> Job {
            Job {
                ops: vec![Op {
                    layer: 0,
                    kind: OpKind::Line,
                    speed,
                    passes: 1,
                    air_assist: false,
                    z: 0.0,
                    z_per_pass: 0.0,
                    moves: vec![Move::Travel(Point::new(1.0, 1.0)), Move::Cut { to, power }],
                }],
            }
        }

        #[test]
        fn unusable_feed_rates_and_powers_are_refused_not_written() {
            let m = Machine::default();
            for bad in [f64::NAN, f64::INFINITY, 0.0, -100.0, 1e12] {
                assert!(matches!(generate(&job(bad, 50.0, Point::new(5.0, 5.0)), &m, Placement::Absolute), Err(GcodeError::BadFeed(_))), "{bad}");
            }
            assert_eq!(generate(&job(1000.0, f64::NAN, Point::new(5.0, 5.0)), &m, Placement::Absolute), Err(GcodeError::NonFinite));
            assert!(generate(&job(1000.0, 50.0, Point::new(5.0, 5.0)), &m, Placement::Absolute).is_ok());
        }

        #[test]
        fn relative_jobs_need_a_sane_extent_and_moves_that_match_it() {
            let m = Machine::default();
            let at = |extent| Placement::Relative { anchor: Point::new(0.0, 0.0), extent, set_zero: false };
            assert_eq!(
                generate(&job(1000.0, 50.0, Point::new(5.0, 5.0)), &m, at((-5.0, 10.0))),
                Err(GcodeError::NonFinite),
                "a negative extent is nonsense"
            );
            assert_eq!(
                generate(&job(1000.0, 50.0, Point::new(5000.0, 0.0)), &m, at((10.0, 10.0))),
                Err(GcodeError::OutsideExtent),
                "a move far beyond the declared extent means the extent is wrong"
            );
            assert!(
                generate(&job(1000.0, 50.0, Point::new(-30.0, 40.0)), &m, at((10.0, 10.0))).is_ok(),
                "lead-ins may leave the job's own bounds a little"
            );
        }
    }

    mod rotary_tests {
        use super::*;

        fn y_words(rotary: laserfoundry_doc::Rotary) -> Vec<String> {
            let mut doc = Document::default();
            doc.machine.origin = Origin::TopLeft;
            doc.machine.rotary = rotary;
            doc.add_shape(Shape::path("a", 0, rect_path(10.0, 10.0, 20.0, 40.0)));
            let g = generate(&plan(&doc).unwrap(), &doc.machine, Placement::Absolute).unwrap();
            g.lines().filter_map(|l| l.split_whitespace().find(|w| w.starts_with('Y')).map(String::from)).collect()
        }

        #[test]
        fn y_is_scaled_by_the_reference_over_the_object_diameter() {
            let plain = y_words(laserfoundry_doc::Rotary::default());
            assert!(plain.contains(&"Y50".to_string()), "{plain:?}");
            let scaled = y_words(laserfoundry_doc::Rotary { enabled: true, reference_diameter: 60.0, object_diameter: 30.0 });
            assert!(scaled.contains(&"Y100".to_string()) && scaled.contains(&"Y20".to_string()), "{scaled:?}");
            let off = y_words(laserfoundry_doc::Rotary { enabled: false, reference_diameter: 60.0, object_diameter: 30.0 });
            assert_eq!(off, plain);
        }

        #[test]
        fn unusable_diameters_are_refused_not_divided_by() {
            let mut doc = Document::default();
            doc.machine.rotary = laserfoundry_doc::Rotary { enabled: true, reference_diameter: 50.0, object_diameter: 0.0 };
            doc.add_shape(Shape::path("a", 0, rect_path(10.0, 10.0, 20.0, 10.0)));
            assert_eq!(generate(&plan(&doc).unwrap(), &doc.machine, Placement::Absolute), Err(GcodeError::BadMachine("rotary diameters")));
        }
    }

    mod z_axis_tests {
        use super::*;

        fn program(z: f64, step: f64, passes: u32, second_layer: bool) -> String {
            let mut doc = Document::default();
            if let Some(l) = doc.layer_mut(0) {
                l.z_offset = z;
                l.z_per_pass = step;
                l.passes = passes;
            }
            doc.add_shape(Shape::path("a", 0, rect_path(10.0, 10.0, 20.0, 10.0)));
            if second_layer {
                doc.add_shape(Shape::path("b", 1, rect_path(50.0, 10.0, 20.0, 10.0)));
            }
            generate(&plan(&doc).unwrap(), &doc.machine, Placement::Absolute).unwrap()
        }

        #[test]
        fn z_is_left_alone_unless_a_layer_asks_for_it() {
            assert!(!program(0.0, 0.0, 2, true).contains(" Z"));
        }

        #[test]
        fn each_pass_steps_z_and_later_layers_return_to_zero() {
            let g = program(-2.0, -0.5, 3, true);
            let zs: Vec<&str> = g.lines().filter(|l| l.contains(" Z")).collect();
            assert_eq!(zs, vec!["G0 Z-2", "G0 Z-2.5", "G0 Z-3", "G0 Z0"], "{g}");
            let first_z = g.lines().position(|l| l == "G0 Z-2").unwrap();
            let first_cut = g.lines().position(|l| l.starts_with("G1")).unwrap();
            assert!(first_z < first_cut, "Z is set before the cut starts");
        }

        #[test]
        fn absurd_z_values_are_refused_by_the_planner() {
            let mut doc = Document::default();
            doc.add_shape(Shape::path("a", 0, rect_path(10.0, 10.0, 20.0, 10.0)));
            for bad in [f64::NAN, 1e9, -500.0] {
                if let Some(l) = doc.layer_mut(0) {
                    l.z_offset = bad;
                }
                assert!(plan(&doc).is_err(), "{bad}");
            }
        }
    }

    mod dialect_tests {
        use laserfoundry_doc::Dialect;

        use super::*;

        fn program(dialect: Dialect, max_s: u32) -> String {
            let mut doc = Document::default();
            doc.machine.dialect = dialect;
            doc.machine.max_s = max_s;
            if let Some(l) = doc.layer_mut(0) {
                l.max_power = 25.0;
                l.speed = 900.0;
            }
            doc.add_shape(Shape::path("r", 0, rect_path(10.0, 10.0, 20.0, 10.0)));
            generate(&plan(&doc).unwrap(), &doc.machine, Placement::Absolute).unwrap()
        }

        #[test]
        fn marlin_uses_inline_power_in_its_configured_units() {
            let g = program(Dialect::Marlin, 255);
            assert!(g.contains("\nM3 I S0\n"), "{g}");
            assert!(!g.contains("M4"), "{g}");
            assert!(g.contains(" S64"), "25% of 255 is 64: {g}");
            assert!(g.trim_end().ends_with("G0 X0 Y0") && g.contains("\nM5\n"), "{g}");
        }

        #[test]
        fn smoothie_uses_fractional_s_on_each_move_and_no_spindle_on() {
            let g = program(Dialect::Smoothie, 1000);
            assert!(!g.contains("M3") && !g.contains("M4"), "the laser must only fire on G1 moves: {g}");
            assert!(g.contains("G1 X30 S0.25 F900") || g.contains(" S0.25"), "{g}");
            assert!(g.contains("\nM5\n"), "{g}");
        }

        #[test]
        fn grbl_output_is_unchanged_and_the_default() {
            assert_eq!(Machine::default().dialect, Dialect::Grbl);
            let g = program(Dialect::Grbl, 1000);
            assert!(g.contains("\nM4 S0\n") && g.contains(" S250"), "{g}");
        }

        #[test]
        fn every_dialect_still_refuses_out_of_bed_jobs() {
            for d in Dialect::ALL {
                let mut doc = Document::default();
                doc.machine.dialect = d;
                doc.add_shape(Shape::path("r", 0, rect_path(1000.0, 10.0, 20.0, 10.0)));
                assert!(matches!(generate(&plan(&doc).unwrap(), &doc.machine, Placement::Absolute), Err(GcodeError::OutOfBed { .. })), "{d:?}");
            }
        }
    }

    mod program_tests {
        use super::*;

        #[test]
        fn rectangle_program_matches_expected_text() {
            let g = rect_program(Origin::TopLeft);
            let expected = "\
; LaserFoundry job
G21
G90
M4 S0
; layer 0 pass 1
G0 X10 Y10
G1 X30 S500 F1200
G1 Y20
G1 X10
G1 Y10
M5
G0 X0 Y0
";
            assert_eq!(g, expected);
        }

        #[test]
        fn bottom_left_origin_flips_y() {
            let g = rect_program(Origin::BottomLeft);
            assert!(g.contains("G0 X10 Y90"), "{g}");
            assert!(g.contains("G1 Y80"), "{g}");
        }

        #[test]
        fn passes_repeat_and_air_assist_wraps() {
            let mut doc = Document::default();
            if let Some(l) = doc.layer_mut(0) {
                l.passes = 2;
                l.air_assist = true;
            }
            doc.add_shape(Shape::path("r", 0, rect_path(0.0, 0.0, 5.0, 5.0)));
            let g = generate(&plan(&doc).unwrap(), &doc.machine, Placement::Absolute).unwrap();
            assert_eq!(g.matches("pass").count(), 2);
            assert!(g.contains("M8") && g.contains("M9"));
        }

        #[test]
        fn constant_power_mode_uses_m3() {
            let mut doc = Document::default();
            doc.machine.dynamic_power = false;
            assert!(generate(&Job::default(), &doc.machine, Placement::Absolute).unwrap().contains("M3 S0"));
        }
    }

    mod safety_tests {
        use super::*;

        #[test]
        fn out_of_bed_is_refused() {
            let mut doc = Document::default();
            doc.add_shape(Shape::path("r", 0, rect_path(390.0, 0.0, 50.0, 5.0)));
            let err = generate(&plan(&doc).unwrap(), &doc.machine, Placement::Absolute).unwrap_err();
            assert!(matches!(err, GcodeError::OutOfBed { .. }), "{err}");
        }

        #[test]
        fn non_finite_coordinates_are_refused() {
            let job = Job {
                ops: vec![laserfoundry_toolpath::Op {
                    layer: 0,
                    kind: laserfoundry_toolpath::OpKind::Line,
                    speed: 100.0,
                    passes: 1,
                    air_assist: false,
                    z: 0.0,
                    z_per_pass: 0.0,
                    moves: vec![Move::Travel(Point::new(f64::NAN, 0.0))],
                }],
            };
            assert_eq!(generate(&job, &Machine::default(), Placement::Absolute), Err(GcodeError::NonFinite));
        }

        #[test]
        fn broken_machine_profiles_are_refused() {
            let m = Machine { bed_width: 0.0, ..Machine::default() };
            assert!(generate(&Job::default(), &m, Placement::Absolute).is_err());
            let m = Machine { max_s: 0, ..Machine::default() };
            assert!(generate(&Job::default(), &m, Placement::Absolute).is_err());
        }

        #[test]
        fn layer_names_cannot_inject_gcode_through_comments() {
            let machine = Machine::default();
            let mut w = Writer::new(&machine, Placement::Absolute).unwrap();
            w.comment("evil\nM3 S1000");
            assert!(!w.out.contains("\nM3"));
        }

        #[test]
        fn zero_is_never_negative_zero() {
            assert_eq!(num(-0.0001), "0");
            assert_eq!(num(12.5), "12.5");
            assert_eq!(num(3.0), "3");
        }
    }

    mod placement_tests {
        use super::*;

        fn job_at(x: f64, y: f64) -> (Document, Job) {
            let mut doc = Document::default();
            doc.machine.origin = Origin::TopLeft;
            doc.add_shape(Shape::path("r", 0, rect_path(x, y, 20.0, 10.0)));
            let job = plan(&doc).unwrap();
            (doc, job)
        }

        #[test]
        fn relative_placement_zeroes_the_anchor_and_can_set_work_zero() {
            let (doc, job) = job_at(100.0, 50.0);
            let p = Placement::Relative { anchor: Point::new(100.0, 50.0), extent: (20.0, 10.0), set_zero: true };
            let g = generate(&job, &doc.machine, p).unwrap();
            assert!(g.contains("G92 X0 Y0"), "{g}");
            assert!(
                g.contains(
                    "G0 X0 Y0
G1 X20 S"
                ),
                "{g}"
            );
            assert!(!g.contains("X100"), "{g}");
        }

        #[test]
        fn user_origin_does_not_emit_g92() {
            let (doc, job) = job_at(5.0, 5.0);
            let p = Placement::Relative { anchor: Point::new(5.0, 5.0), extent: (20.0, 10.0), set_zero: false };
            assert!(!generate(&job, &doc.machine, p).unwrap().contains("G92"));
        }

        #[test]
        fn relative_y_flips_for_a_bottom_left_machine() {
            let (mut doc, job) = job_at(0.0, 0.0);
            doc.machine.origin = Origin::BottomLeft;
            let p = Placement::Relative { anchor: Point::new(0.0, 0.0), extent: (20.0, 10.0), set_zero: false };
            let g = generate(&job, &doc.machine, p).unwrap();
            assert!(g.contains("Y-10"), "{g}");
        }

        #[test]
        fn relative_jobs_may_sit_anywhere_but_must_fit_the_bed() {
            let (doc, job) = job_at(5000.0, 0.0); // far outside the bed in design space, fine when relative
            let ok = Placement::Relative { anchor: Point::new(5000.0, 0.0), extent: (20.0, 10.0), set_zero: false };
            assert!(generate(&job, &doc.machine, ok).is_ok());
            let too_big = Placement::Relative { anchor: Point::ORIGIN, extent: (500.0, 10.0), set_zero: false };
            assert!(matches!(generate(&job, &doc.machine, too_big), Err(GcodeError::JobLargerThanBed { .. })));
            let nan = Placement::Relative { anchor: Point::new(f64::NAN, 0.0), extent: (1.0, 1.0), set_zero: false };
            assert_eq!(generate(&job, &doc.machine, nan), Err(GcodeError::NonFinite));
        }
    }

    mod frame_tests {
        use super::*;

        #[test]
        fn frame_traces_bounds_with_requested_power() {
            let g = frame(
                Rect::new(10.0, 10.0, 30.0, 20.0),
                &Machine { origin: Origin::TopLeft, ..Machine::default() },
                Placement::Absolute,
                3000.0,
                1.0,
            )
            .unwrap();
            assert!(g.contains("S10"), "{g}");
            assert_eq!(g.matches("G1").count(), 4);
        }
    }
}
