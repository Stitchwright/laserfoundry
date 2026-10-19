use laserfoundry_doc::{Layer, LayerId, LayerMode, MAX_PRESETS, Preset};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, parse};
use crate::{EngineError, Session};

/// Longest preset name kept, in characters.
const MAX_NAME: usize = 60;

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "preset.save",
        label: "Save Cut Settings as Preset",
        menu: "Laser",
        shortcut: None,
        params_doc: "{name, layer}  — stores the layer's cut settings under that name (replacing a preset of the same name)",
        mutates: true,
        run: save,
    },
    CommandSpec {
        id: "preset.apply",
        label: "Apply Preset",
        menu: "Laser",
        shortcut: None,
        params_doc: "{name, layer}  — copies the preset's cut settings onto the layer (its name, colour and output switch stay)",
        mutates: true,
        run: apply,
    },
    CommandSpec {
        id: "preset.add_starters",
        label: "Add Starter Presets",
        menu: "Laser",
        shortcut: None,
        params_doc: "{}  — adds generic starting points (by operation, not by material) that are not already in the project; always check them with a material test grid",
        mutates: true,
        run: add_starters,
    },
    CommandSpec {
        id: "preset.export",
        label: "Export Presets",
        menu: "",
        shortcut: None,
        params_doc: "{}  — returns every preset in the project as {presets: [...]}, for keeping in a library shared between projects",
        mutates: false,
        run: export,
    },
    CommandSpec {
        id: "preset.import",
        label: "Import Presets",
        menu: "",
        shortcut: None,
        params_doc: "{presets: [...]}  — adds presets from a library; names already in the project are kept as they are",
        mutates: true,
        run: import,
    },
    CommandSpec { id: "preset.delete", label: "Delete Preset", menu: "Laser", shortcut: None, params_doc: "{name}", mutates: true, run: delete },
];

#[derive(Deserialize)]
struct NamedLayer {
    name: String,
    layer: LayerId,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

fn clean(name: &str) -> Result<String, EngineError> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    if name.is_empty() { Err(EngineError::BadParams("a preset needs a name".into())) } else { Ok(name) }
}

fn save(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: NamedLayer = parse(params)?;
    let name = clean(&p.name)?;
    let settings = s.doc.layer(p.layer).ok_or_else(|| EngineError::NotFound(format!("layer {} does not exist", p.layer)))?.clone();
    match s.doc.presets.iter_mut().find(|x| x.name == name) {
        Some(existing) => existing.settings = settings,
        None => {
            if s.doc.presets.len() >= MAX_PRESETS {
                return Err(EngineError::BadParams(format!("at most {MAX_PRESETS} presets can be kept")));
            }
            s.doc.presets.push(Preset { name: name.clone(), settings });
        }
    }
    Ok(json!({ "name": name, "count": s.doc.presets.len() }))
}

fn apply(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: NamedLayer = parse(params)?;
    let name = clean(&p.name)?;
    let preset =
        s.doc.presets.iter().find(|x| x.name == name).ok_or_else(|| EngineError::NotFound(format!("no preset called {name}")))?.settings.clone();
    let layer = s.doc.layer_mut(p.layer).ok_or_else(|| EngineError::NotFound(format!("layer {} does not exist", p.layer)))?;
    layer.apply_settings(&preset);
    Ok(json!({ "layer": p.layer }))
}

fn delete(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: Named = parse(params)?;
    let name = clean(&p.name)?;
    let before = s.doc.presets.len();
    s.doc.presets.retain(|x| x.name != name);
    if s.doc.presets.len() == before {
        return Err(EngineError::NotFound(format!("no preset called {name}")));
    }
    Ok(json!({ "count": s.doc.presets.len() }))
}

/// Generic starting points, named for what they do rather than for a material: how a given sheet
/// responds depends on the laser, its optics and the material, so these only give a first guess to
/// refine with the material test grid. Written for this project, not taken from anyone's library.
const STARTERS: &[(&str, LayerMode, f64, f64, f64, u32, f64)] = &[
    // name, mode, speed mm/min, max power %, min power %, passes, interval mm
    ("Starter: light mark", LayerMode::Line, 3000.0, 15.0, 5.0, 1, 0.1),
    ("Starter: engrave (fill)", LayerMode::Fill, 3000.0, 35.0, 10.0, 1, 0.1),
    ("Starter: deep engrave (fill)", LayerMode::Fill, 1500.0, 60.0, 15.0, 2, 0.08),
    ("Starter: photo (image)", LayerMode::Fill, 4000.0, 40.0, 5.0, 1, 0.1),
    ("Starter: thin cut", LayerMode::Line, 600.0, 90.0, 40.0, 1, 0.1),
    ("Starter: multi-pass cut", LayerMode::Line, 900.0, 100.0, 50.0, 4, 0.1),
];

fn add_starters(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    let mut added = 0;
    for (name, mode, speed, max_power, min_power, passes, interval) in STARTERS {
        if s.doc.presets.iter().any(|p| p.name == *name) {
            continue;
        }
        if s.doc.presets.len() >= MAX_PRESETS {
            return Err(EngineError::BadParams(format!("at most {MAX_PRESETS} presets can be kept")));
        }
        let mut settings = Layer::numbered(0);
        settings.mode = *mode;
        settings.speed = *speed;
        settings.max_power = *max_power;
        settings.min_power = *min_power;
        settings.passes = *passes;
        settings.interval = *interval;
        s.doc.presets.push(Preset { name: (*name).to_string(), settings });
        added += 1;
    }
    Ok(json!({ "added": added, "count": s.doc.presets.len() }))
}

fn export(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    Ok(json!({ "presets": s.doc.presets }))
}

#[derive(Deserialize)]
struct ImportParams {
    presets: Vec<Preset>,
}

fn import(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: ImportParams = parse(params)?;
    let mut added = 0;
    for mut preset in p.presets {
        preset.name = clean(&preset.name)?;
        if s.doc.presets.iter().any(|x| x.name == preset.name) {
            continue;
        }
        if s.doc.presets.len() >= MAX_PRESETS {
            return Err(EngineError::BadParams(format!("at most {MAX_PRESETS} presets can be kept")));
        }
        // A library entry could come from anywhere: refuse settings the planner would reject.
        let l = &preset.settings;
        let ok =
            l.speed.is_finite() && l.speed > 0.0 && l.interval.is_finite() && l.interval > 0.0 && l.min_power.is_finite() && l.max_power.is_finite();
        if !ok {
            return Err(EngineError::BadParams(format!("preset {} has unusable settings", preset.name)));
        }
        s.doc.presets.push(preset);
        added += 1;
    }
    Ok(json!({ "added": added, "count": s.doc.presets.len() }))
}
