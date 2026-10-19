//! Headless LaserFoundry: batch conversion and a JSON command interface.
//!
//! ```text
//! laserfoundry-cli gcode design.svg -o job.gcode [--layer 0:speed=900,max_power=40] [--bed 300x200] [--origin top_left]
//! laserfoundry-cli info design.svg
//! laserfoundry-cli commands
//! laserfoundry-cli mcp                        # Model Context Protocol server on stdin/stdout
//! laserfoundry-cli mcp --connect              # the same, but editing the drawing open in a running `laserfoundry --agent-link`
//! laserfoundry-cli run script.json            # [{"id": "...", "params": {...}}, ...] then prints the last result
//! laserfoundry-cli fonts download [--all] [--to DIR]   # fetch the font library (the Foundry fonts; --all adds Ink/Stitch's)
//! laserfoundry-cli translation-template       # every text this tool prints, as a catalog to translate
//! ```
//!
//! Output is translated with the same catalog as the desktop app (`translation.json` in the
//! LaserFoundry data folder), or the one named by `--translation FILE` or `$LASERFOUNDRY_TRANSLATION`.

use std::path::Path;
use std::process::ExitCode;

use laserfoundry_engine::{Session, commands};
use serde_json::{Value, json};

/// Every fixed text the tool prints, so a translator can be given all of them (`translation-template`).
const USAGE_LINES: &[&str] = &[
    "usage: laserfoundry-cli [--translation FILE] <command>",
    "",
    "  gcode <input.svg|dxf|png|jpg|laserfoundry> -o <out.gcode> [--bed WxH] [--origin top_left|bottom_left]",
    "                       [--start-from absolute|user_origin|current_position] [--anchor center|top_left|...]",
    "                       [--set LAYER:key=value,key=value]...",
    "  info <input>         shape count, size, estimated time",
    "  commands             list every engine command with its parameters",
    "  mcp [--connect]      serve the engine's commands as MCP tools on stdin/stdout (for AI agents);",
    "                       --connect works in the window of a running `laserfoundry --agent-link` instead",
    "  run <script.json>    run a JSON array of {\"id\", \"params\"} commands and print the last result",
    "  fonts download [--all] [--to DIR]",
    "                       download the font library (Stitchwright/stitchfoundry-fonts) to the data folder or DIR;",
    "                       --all adds the large Ink/Stitch collection",
    "  fonts path           print the folder the font library is kept in",
    "  translation-template print every text this tool shows as a catalog to translate",
];

mod msg {
    pub const ERROR: &str = "error: {message}";
    pub const FILE_ERROR: &str = "{path}: {error}";
    pub const SHAPES: &str = "shapes: {count}";
    pub const SIZE: &str = "size: {size}";
    pub const SIZE_EMPTY: &str = "empty";
    pub const SIZE_MM: &str = "{width} × {height} mm";
    pub const ESTIMATE: &str = "estimated time: {seconds} s (cut {cut} mm, travel {travel} mm)";
    pub const WROTE: &str = "wrote {path} (estimated {seconds} s)";
    pub const BED_FORMAT: &str = "--bed expects WIDTHxHEIGHT, e.g. 300x200";
    pub const BAD_BED_WIDTH: &str = "bad bed width";
    pub const BAD_BED_HEIGHT: &str = "bad bed height";
    pub const NEEDS_VALUE: &str = "{flag} needs a value";
    pub const UNKNOWN_OPTION: &str = "unknown option {option}";
    pub const MISSING_OUTPUT: &str = "missing -o <out.gcode>";
    pub const SET_FORMAT: &str = "--set expects LAYER:key=value,...";
    pub const LAYER_NUMBER: &str = "layer must be a number";
    pub const SET_PAIRS: &str = "--set expects key=value pairs";
    pub const MISSING_ID: &str = "step {step}: missing \"id\"";
    pub const STEP_FAILED: &str = "step {step} ({id}): {error}";
    pub const NO_LINK_FOLDER: &str = "no folder for the link file";
    pub const FONTS_PROGRESS: &str = "fonts: {done} of {total}";
    pub const FONTS_DONE: &str = "font library in {path}: {fetched} downloaded, {kept} already there ({bytes} bytes)";
    pub const FONTS_NOTE: &str =
        "The Foundry fonts work for text: give a folder under fonts/ as the font. The Ink/Stitch fonts are not drawable yet.";
    pub const NO_FONT_FOLDER: &str = "no folder to keep the fonts in; use --to DIR";
    pub const FONTS_FAILED: &str = "{reason}";
    pub const TRANSLATION_FAILED: &str = "translation {path}: {error}";

    pub const ALL: &[&str] = &[
        ERROR,
        FILE_ERROR,
        SHAPES,
        SIZE,
        SIZE_EMPTY,
        SIZE_MM,
        ESTIMATE,
        WROTE,
        BED_FORMAT,
        BAD_BED_WIDTH,
        BAD_BED_HEIGHT,
        NEEDS_VALUE,
        UNKNOWN_OPTION,
        MISSING_OUTPUT,
        SET_FORMAT,
        LAYER_NUMBER,
        SET_PAIRS,
        MISSING_ID,
        STEP_FAILED,
        NO_LINK_FOLDER,
        FONTS_PROGRESS,
        FONTS_DONE,
        FONTS_NOTE,
        NO_FONT_FOLDER,
        FONTS_FAILED,
        TRANSLATION_FAILED,
    ];
}

use laserfoundry_i18n::{tr, tr_fill, tr_message};

/// A fixed text, translated.
fn t(text: &str) -> String {
    tr(text)
}

/// A text with `{name}` places, translated and filled in.
fn tf(template: &str, args: &[(&str, String)]) -> String {
    tr_fill(template, args)
}

/// An engine error, translated by template.
fn engine(e: impl std::fmt::Display) -> String {
    tr_message(&e.to_string())
}

fn usage() -> String {
    USAGE_LINES.iter().map(|l| if l.is_empty() { String::new() } else { t(l) }).collect::<Vec<_>>().join("\n") + "\n"
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let loaded = select_translation(&mut args);
    let result = loaded.and_then(|()| run(&args));
    match result {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", tf(msg::ERROR, &[("message", e)]));
            ExitCode::FAILURE
        }
    }
}

/// Activates the catalog: `--translation FILE` (which must load), else `$LASERFOUNDRY_TRANSLATION`, else
/// the desktop app's file if there is one (a missing or unreadable default is ignored).
fn select_translation(args: &mut Vec<String>) -> Result<(), String> {
    let mut explicit = std::env::var_os("LASERFOUNDRY_TRANSLATION").filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    if args.first().map(String::as_str) == Some("--translation") {
        args.remove(0);
        if args.is_empty() {
            return Err(tf(msg::NEEDS_VALUE, &[("flag", "--translation".into())]));
        }
        explicit = Some(args.remove(0).into());
    }
    let (path, required) = match explicit {
        Some(p) => (Some(p), true),
        None => (laserfoundry_i18n::default_file().filter(|p| p.exists()), false),
    };
    let Some(path) = path else { return Ok(()) };
    match laserfoundry_i18n::load(&path) {
        Ok(translation) => laserfoundry_i18n::set_active(Some(translation)),
        Err(e) if required => return Err(tf(msg::TRANSLATION_FAILED, &[("path", path.display().to_string()), ("error", e)])),
        Err(_) => {}
    }
    Ok(())
}

fn run(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("commands") => Ok(commands().map(|c| format!("{:<22} {:<28} {}", c.id, c.label, c.params_doc)).collect::<Vec<_>>().join("\n")),
        Some("info") => info(args.get(1).ok_or_else(usage)?),
        Some("gcode") => gcode(&args[1..]),
        Some("run") => script(args.get(1).ok_or_else(usage)?),
        Some("mcp") => mcp(args.get(1).map(String::as_str)),
        Some("fonts") => fonts(&args[1..]),
        Some("translation-template") => Ok(translation_template()),
        _ => Err(usage()),
    }
}

/// `fonts download [--all] [--to DIR]` and `fonts path`.
fn fonts(args: &[String]) -> Result<String, String> {
    let mut set = laserfoundry_fontlib::Set::Foundry;
    let mut dest = None;
    let mut source = laserfoundry_fontlib::Source::default();
    match args.first().map(String::as_str) {
        Some("path") => return laserfoundry_fontlib::default_dir().map(|d| d.display().to_string()).ok_or_else(|| t(msg::NO_FONT_FOLDER)),
        Some("download") => {}
        _ => return Err(usage()),
    }
    let mut i = 1;
    while let Some(flag) = args.get(i) {
        let value = || args.get(i + 1).cloned().ok_or_else(|| tf(msg::NEEDS_VALUE, &[("flag", flag.clone())]));
        match flag.as_str() {
            "--all" => set = laserfoundry_fontlib::Set::Everything,
            "--to" => {
                dest = Some(std::path::PathBuf::from(value()?));
                i += 1;
            }
            // For a mirror of the library (and for tests).
            "--tree-url" => {
                source.tree_url = value()?;
                i += 1;
            }
            "--raw-base" => {
                source.raw_base = value()?;
                i += 1;
            }
            other => return Err(tf(msg::UNKNOWN_OPTION, &[("option", other.into())])),
        }
        i += 1;
    }
    let dest = dest.or_else(laserfoundry_fontlib::default_dir).ok_or_else(|| t(msg::NO_FONT_FOLDER))?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut shown = 0;
    let summary = laserfoundry_fontlib::download(&dest, &source, set, &cancel, &mut |done, total| {
        // Progress goes to stderr every so often so stdout stays the result.
        if done == total || done >= shown + 25 {
            shown = done;
            eprintln!("{}", tf(msg::FONTS_PROGRESS, &[("done", done.to_string()), ("total", total.to_string())]));
        }
    })
    .map_err(|e| tf(msg::FONTS_FAILED, &[("reason", engine(e))]))?;
    Ok(format!(
        "{}\n{}",
        tf(
            msg::FONTS_DONE,
            &[
                ("path", dest.display().to_string()),
                ("fetched", summary.fetched.to_string()),
                ("kept", summary.kept.to_string()),
                ("bytes", summary.bytes.to_string())
            ]
        ),
        t(msg::FONTS_NOTE)
    ))
}

/// A catalog of every text this tool shows, with the interface's own strings left out.
fn translation_template() -> String {
    for text in USAGE_LINES.iter().chain(msg::ALL) {
        let _ = tr(text);
    }
    laserfoundry_i18n::template()
}

fn load(path: &str) -> Result<Session, String> {
    let mut s = Session::new();
    let id = match Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("svg") => "file.import_svg",
        Some("dxf") => "file.import_dxf",
        Some("png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp") => "file.import_image",
        _ => "file.open",
    };
    s.execute(id, &json!({ "path": path })).map_err(|e| tf(msg::FILE_ERROR, &[("path", path.into()), ("error", engine(e))]))?;
    Ok(s)
}

fn info(path: &str) -> Result<String, String> {
    let mut s = load(path)?;
    let est = s.execute("job.estimate", &Value::Null).map_err(engine)?;
    let b = s.doc.bounds_of(&s.doc.shapes.iter().map(|sh| sh.id).collect::<Vec<_>>());
    let size = b.map_or_else(
        || t(msg::SIZE_EMPTY),
        |b| tf(msg::SIZE_MM, &[("width", format!("{:.2}", b.width())), ("height", format!("{:.2}", b.height()))]),
    );
    let number = |key: &str| format!("{:.0}", est[key].as_f64().unwrap_or(0.0));
    Ok([
        tf(msg::SHAPES, &[("count", s.doc.shapes.len().to_string())]),
        tf(msg::SIZE, &[("size", size)]),
        tf(msg::ESTIMATE, &[("seconds", number("seconds")), ("cut", number("cut_mm")), ("travel", number("travel_mm"))]),
    ]
    .join("\n"))
}

fn gcode(args: &[String]) -> Result<String, String> {
    let input = args.first().ok_or_else(usage)?;
    let mut s = load(input)?;
    let mut out_path = None;
    let mut i = 1;
    while let Some(flag) = args.get(i) {
        let value = args.get(i + 1).ok_or_else(|| tf(msg::NEEDS_VALUE, &[("flag", flag.clone())]))?;
        match flag.as_str() {
            "-o" => out_path = Some(value.clone()),
            "--bed" => {
                let (w, h) = value.split_once('x').ok_or_else(|| t(msg::BED_FORMAT))?;
                let (w, h): (f64, f64) = (w.parse().map_err(|_| t(msg::BAD_BED_WIDTH))?, h.parse().map_err(|_| t(msg::BAD_BED_HEIGHT))?);
                s.execute("machine.set", &json!({ "bed_width": w, "bed_height": h })).map_err(engine)?;
            }
            "--origin" => {
                s.execute("machine.set", &json!({ "origin": value })).map_err(engine)?;
            }
            "--start-from" => {
                s.execute("job.set_origin", &json!({ "start_from": value })).map_err(engine)?;
            }
            "--anchor" => {
                s.execute("job.set_origin", &json!({ "anchor": value })).map_err(engine)?;
            }
            "--set" => apply_layer_settings(&mut s, value)?,
            other => return Err(tf(msg::UNKNOWN_OPTION, &[("option", other.into())])),
        }
        i += 2;
    }
    let out_path = out_path.ok_or_else(|| t(msg::MISSING_OUTPUT))?;
    s.execute("file.export_gcode", &json!({ "path": out_path })).map_err(engine)?;
    let est = s.execute("job.estimate", &Value::Null).map_err(engine)?;
    Ok(tf(msg::WROTE, &[("path", out_path), ("seconds", format!("{:.0}", est["seconds"].as_f64().unwrap_or(0.0)))]))
}

/// `3:speed=900,max_power=40,mode=fill`
fn apply_layer_settings(s: &mut Session, spec: &str) -> Result<(), String> {
    let (layer, rest) = spec.split_once(':').ok_or_else(|| t(msg::SET_FORMAT))?;
    let mut params = serde_json::Map::new();
    params.insert("id".into(), json!(layer.parse::<u32>().map_err(|_| t(msg::LAYER_NUMBER))?));
    for pair in rest.split(',') {
        let (k, v) = pair.split_once('=').ok_or_else(|| t(msg::SET_PAIRS))?;
        let value = v.parse::<f64>().map(|n| json!(n)).or_else(|_| v.parse::<bool>().map(|b| json!(b))).unwrap_or_else(|_| json!(v));
        params.insert(k.into(), value);
    }
    s.execute("layer.set", &Value::Object(params)).map(|_| ()).map_err(engine)
}

fn script(path: &str) -> Result<String, String> {
    let file_error = |e: &dyn std::fmt::Display| tf(msg::FILE_ERROR, &[("path", path.into()), ("error", e.to_string())]);
    let text = std::fs::read_to_string(path).map_err(|e| file_error(&e))?;
    let steps: Vec<Value> = serde_json::from_str(&text).map_err(|e| file_error(&e))?;
    let mut s = Session::new();
    let mut last = Value::Null;
    for (n, step) in steps.iter().enumerate() {
        let id = step["id"].as_str().ok_or_else(|| tf(msg::MISSING_ID, &[("step", (n + 1).to_string())]))?;
        last = s
            .execute(id, &step["params"])
            .map_err(|e| tf(msg::STEP_FAILED, &[("step", (n + 1).to_string()), ("id", id.into()), ("error", engine(e))]))?;
    }
    serde_json::to_string_pretty(&last).map_err(|e| e.to_string())
}

/// Speaks MCP on stdin/stdout until the client closes the pipe. Nothing else may print to stdout.
fn mcp(flag: Option<&str>) -> Result<String, String> {
    match flag {
        None => {}
        Some("--connect") => {
            let file = laserfoundry_mcp::link::default_link_file().ok_or_else(|| t(msg::NO_LINK_FOLDER))?;
            laserfoundry_mcp::link::proxy(&file, std::io::stdin().lock(), std::io::stdout()).map_err(engine)?;
            return Ok(String::new());
        }
        Some(_) => return Err(usage()),
    }
    let mut s = Session::new();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stdout().lock();
    laserfoundry_mcp::serve(&mut s, &mut input, &mut output).map_err(engine)?;
    Ok(String::new())
}
