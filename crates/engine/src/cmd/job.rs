use laserfoundry_device::{SimulatedGrbl, StreamOptions, stream_program};
use laserfoundry_doc::{Anchor, StartFrom};
use laserfoundry_gcode::Placement;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, parse};
use crate::{EngineError, Session};

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "job.gcode",
        label: "Generate G-code",
        menu: "Laser",
        shortcut: None,
        params_doc: "{include?: bool}  — stores the program in the session; include=true returns the text",
        mutates: false,
        run: gcode,
    },
    CommandSpec {
        id: "job.set_origin",
        label: "Start From / Job Origin",
        menu: "Laser",
        shortcut: None,
        params_doc: "{start_from?: absolute|user_origin|current_position, anchor?: top_left|top|top_right|left|center|right|bottom_left|bottom|bottom_right}",
        mutates: true,
        run: set_origin,
    },
    CommandSpec {
        id: "job.resume",
        label: "Resume Job From Line",
        menu: "Laser",
        shortcut: None,
        params_doc: "{line (1-based, in the generated program), include?: bool}  — builds a program that starts at that line: laser off, moves to where the head would be, restores feed and power. Returns the text with include=true",
        mutates: false,
        run: resume,
    },
    CommandSpec {
        id: "job.set_scope",
        label: "Cut Selected Shapes Only",
        menu: "Laser",
        shortcut: None,
        params_doc: "{selection_only: bool}  — when true, generate/estimate/frame/preview use only the selected shapes (a session setting, not saved in the file)",
        mutates: false,
        run: set_scope,
    },
    CommandSpec {
        id: "job.set_optimize",
        label: "Cut Order Options",
        menu: "Laser",
        shortcut: None,
        params_doc: "{inner_first?: bool, reduce_travel?: bool, remove_duplicates?: bool, by_group?: bool, start?: [x, y] (where ordering starts, mm) or default_start: true}",
        mutates: true,
        run: set_optimize,
    },
    CommandSpec { id: "job.estimate", label: "Estimate Time", menu: "Laser", shortcut: None, params_doc: "{}", mutates: false, run: estimate },
    CommandSpec {
        id: "job.frame",
        label: "Frame",
        menu: "Laser",
        shortcut: None,
        params_doc: "{speed? (mm/min, default 3000), power? (%, default 0)}  — returns a program tracing the job bounds",
        mutates: false,
        run: frame,
    },
    CommandSpec {
        id: "job.simulate",
        label: "Simulate on Virtual Machine",
        menu: "Laser",
        shortcut: None,
        params_doc: "{}  — streams the program to a simulated GRBL controller",
        mutates: false,
        run: simulate,
    },
];

/// Plans the document and renders it as G-code.
pub(super) fn generate(s: &Session) -> Result<String, EngineError> {
    let doc = s.job_doc();
    let job = laserfoundry_toolpath::plan(&doc)?;
    Ok(laserfoundry_gcode::generate(&job, &doc.machine, placement(s)?)?)
}

/// Machine placement for the document's "start from" setting.
fn placement(s: &Session) -> Result<Placement, EngineError> {
    let origin = s.doc.job_origin;
    if origin.start_from == StartFrom::Absolute {
        return Ok(Placement::Absolute);
    }
    let bounds = laserfoundry_toolpath::job_bounds(&s.job_doc()).ok_or_else(|| EngineError::NotFound("there is nothing to output".into()))?;
    Ok(Placement::Relative {
        anchor: origin.anchor.point(bounds),
        extent: (bounds.width(), bounds.height()),
        set_zero: origin.start_from == StartFrom::CurrentPosition,
    })
}

#[derive(Deserialize)]
struct SetOriginParams {
    start_from: Option<StartFrom>,
    anchor: Option<Anchor>,
}

fn set_origin(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: SetOriginParams = parse(params)?;
    let o = &mut s.doc.job_origin;
    o.start_from = p.start_from.unwrap_or(o.start_from);
    o.anchor = p.anchor.unwrap_or(o.anchor);
    Ok(json!({ "start_from": o.start_from, "anchor": o.anchor }))
}

#[derive(Deserialize)]
struct ResumeParams {
    line: usize,
    #[serde(default)]
    include: bool,
}

fn resume(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: ResumeParams = parse(params)?;
    let program = generate(s)?;
    let text = laserfoundry_gcode::resume_from(&program, p.line).map_err(|e| EngineError::BadParams(e.to_string()))?;
    let at = program.lines().nth(p.line.saturating_sub(1)).unwrap_or("").to_string();
    let summary = json!({ "from_line": p.line, "total_lines": program.lines().count(), "line": at, "lines": text.lines().count() });
    Ok(if p.include { json!({ "summary": summary, "gcode": text }) } else { summary })
}

#[derive(Deserialize)]
struct SetScopeParams {
    selection_only: bool,
}

fn set_scope(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: SetScopeParams = parse(params)?;
    s.job_selection_only = p.selection_only;
    Ok(json!({ "selection_only": s.job_selection_only }))
}

#[derive(Deserialize)]
struct SetOptimizeParams {
    inner_first: Option<bool>,
    reduce_travel: Option<bool>,
    remove_duplicates: Option<bool>,
    by_group: Option<bool>,
    start: Option<[f64; 2]>,
    #[serde(default)]
    default_start: bool,
}

fn set_optimize(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: SetOptimizeParams = parse(params)?;
    let o = &mut s.doc.optimize;
    o.inner_first = p.inner_first.unwrap_or(o.inner_first);
    o.reduce_travel = p.reduce_travel.unwrap_or(o.reduce_travel);
    o.remove_duplicates = p.remove_duplicates.unwrap_or(o.remove_duplicates);
    o.by_group = p.by_group.unwrap_or(o.by_group);
    if p.default_start {
        o.start = None;
    } else if let Some(start) = p.start {
        if !start.iter().all(|v| v.is_finite() && v.abs() <= 1e6) {
            return Err(EngineError::BadParams("start must be two finite numbers".into()));
        }
        o.start = Some(start);
    }
    Ok(
        json!({ "inner_first": o.inner_first, "reduce_travel": o.reduce_travel, "remove_duplicates": o.remove_duplicates, "by_group": o.by_group, "start": o.start }),
    )
}

#[derive(Deserialize, Default)]
struct GcodeParams {
    #[serde(default)]
    include: bool,
}

fn gcode(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: GcodeParams = parse(params)?;
    let program = generate(s)?;
    let summary = json!({ "lines": program.lines().count(), "bytes": program.len() });
    let out = if p.include { json!({ "summary": summary, "gcode": program }) } else { summary };
    s.last_gcode = Some(program);
    Ok(out)
}

fn estimate(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    let job = laserfoundry_toolpath::plan(&s.job_doc())?;
    let t = laserfoundry_toolpath::Timeline::with_acceleration(&job, s.doc.machine.rapid_speed, s.doc.machine.acceleration);
    let layers: Vec<Value> = t
        .layers()
        .iter()
        .map(|l| {
            let name = s.doc.layer(l.layer).map_or_else(String::new, |x| x.name.clone());
            json!({ "layer": l.layer, "name": name, "seconds": l.seconds, "cut_mm": l.cut_mm, "travel_mm": l.travel_mm })
        })
        .collect();
    Ok(json!({ "seconds": t.total, "cut_mm": t.cut_mm(), "travel_mm": t.travel_mm(), "layers": layers }))
}

#[derive(Deserialize)]
struct FrameParams {
    #[serde(default = "frame_speed")]
    speed: f64,
    #[serde(default)]
    power: f64,
}

fn frame_speed() -> f64 {
    3000.0
}

fn frame(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: FrameParams = parse(params)?;
    let bounds = laserfoundry_toolpath::job_bounds(&s.job_doc()).ok_or_else(|| EngineError::NotFound("there is nothing to frame".into()))?;
    let program = laserfoundry_gcode::frame(bounds, &s.doc.machine, placement(s)?, p.speed, p.power)?;
    Ok(json!({ "gcode": program, "bounds": [bounds.x0, bounds.y0, bounds.x1, bounds.y1] }))
}

fn simulate(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    let program = generate(s)?;
    let mut sim = SimulatedGrbl::new();
    let report = stream_program(&mut sim, &program, &StreamOptions::default(), &std::sync::atomic::AtomicBool::new(false), |_| {})?;
    Ok(json!({ "lines_streamed": report.lines, "peak_rx_bytes": sim.max_buffer_used(), "overran_buffer": sim.overran_buffer() }))
}
