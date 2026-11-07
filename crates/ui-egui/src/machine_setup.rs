//! The machine setup wizard (part 1 of #61): choose the simulator or a serial port, read the controller's settings, test the
//! axis directions with the laser off, then name and save the machine. Nothing is sent to the controller except by a button
//! press, and every step can be skipped. Not verified on real hardware.

use std::collections::BTreeMap;
use std::fmt::Display;

use laserfoundry_device::MachineState;
use laserfoundry_doc::Origin;
use serde_json::{Map, Value, json};

use crate::i18n::{tr, trf};
use crate::platform::Platform;
use crate::{LaserFoundryApp, Target};

/// How far the direction test moves the head, in mm.
const TEST_JOG_MM: f64 = 5.0;
/// Slow, because the head may start anywhere.
const TEST_JOG_FEED: f64 = 1500.0;
/// Largest bed side or `$30` the machine settings accept.
const MAX_SETTING: f64 = 100_000.0;
/// Longest machine name kept (the `machine.set` command cuts it here too).
const MAX_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Step {
    #[default]
    Connect,
    Read,
    Direction,
    Save,
}

#[derive(Debug, Default)]
pub(crate) struct Wizard {
    pub open: bool,
    pub step: Step,
    /// The answer to "did the head move away from you?" for a +Y move.
    pub y_away: Option<bool>,
    /// The answer to "did the head move to the right?" for a +X move.
    pub x_right: Option<bool>,
    pub name: String,
}

impl Wizard {
    /// Opens the wizard at its first step, offering `name` as the machine's name.
    pub fn start(&mut self, name: &str) {
        *self = Wizard { open: true, name: name.to_string(), ..Wizard::default() };
    }
}

/// What a `$$` report says about the machine, where it says something usable.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Controller {
    /// `$130`, `$131`: travel in mm.
    pub bed: Option<(f64, f64)>,
    /// `$30`.
    pub max_s: Option<u32>,
    /// `$22`: homing cycle enabled.
    pub homing: Option<bool>,
    /// `$32`.
    pub laser_mode: Option<bool>,
}

pub(crate) fn read_controller(settings: &BTreeMap<u32, String>) -> Controller {
    let num = |id: u32| settings.get(&id).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    let usable = |v: f64| (1.0..=MAX_SETTING).contains(&v);
    let bed = match (num(130), num(131)) {
        (Some(w), Some(h)) if usable(w) && usable(h) => Some((w, h)),
        _ => None,
    };
    Controller {
        bed,
        max_s: num(30).filter(|s| usable(*s)).map(|s| s.round() as u32),
        homing: num(22).map(|v| v != 0.0),
        laser_mode: num(32).map(|v| v != 0.0),
    }
}

/// The origin corner that fits a +Y move which did (`true`) or did not take the head away from the person at the front.
pub(crate) fn origin_for(y_away: bool) -> Origin {
    if y_away { Origin::BottomLeft } else { Origin::TopLeft }
}

/// `$3` (direction invert mask) with the X bit flipped; `None` when the controller's value is not known or not a mask.
pub(crate) fn flip_x_direction(settings: &BTreeMap<u32, String>) -> Option<f64> {
    let current = settings.get(&3)?.trim().parse::<f64>().ok().filter(|v| v.is_finite() && (0.0..=255.0).contains(v))?;
    Some(f64::from((current as u32) ^ 1))
}

/// One word for the machine's state, as shown at the top of the Machine block.
pub(crate) fn status_label(connected: bool, simulator: bool, state: Option<MachineState>, job_active: bool) -> String {
    if !connected {
        return tr("Disconnected");
    }
    let state = match state {
        None => tr("Connecting…"),
        Some(MachineState::Alarm) => tr("Alarm"),
        Some(_) if job_active => tr("Busy"),
        Some(MachineState::Idle | MachineState::Unknown) => tr("Ready"),
        Some(_) => tr("Busy"),
    };
    if simulator { trf!("Simulator: {state}", state = state) } else { state }
}

/// Whether the wizard should open by itself at launch: the host keeps a note, and there is none yet.
pub(crate) fn should_open_on_launch(platform: &dyn Platform) -> bool {
    platform.setup_file().is_some_and(|p| !p.exists())
}

/// Leaves a note that the wizard was finished or skipped, so it does not open by itself again.
fn mark_done(platform: &dyn Platform) {
    let Some(path) = platform.setup_file() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent); // a failure shows up as the write error below
    }
    if let Err(e) = std::fs::write(&path, "{\"done\":true}") {
        log::warn!("could not write {}: {e}", path.display());
    }
}

/// Opens the wizard from the beginning (the menu and the Machine block).
pub(crate) fn open(app: &mut LaserFoundryApp) {
    let name = app.session.doc.machine.name.clone();
    app.wizard.start(&name);
}

/// Closes the wizard for good: it stays closed at launch from now on.
pub(crate) fn finish(app: &mut LaserFoundryApp) {
    app.wizard.open = false;
    mark_done(app.platform.as_ref());
}

/// Records the answer to the up test and sets the origin corner to match.
pub(crate) fn answer_up(app: &mut LaserFoundryApp, away: bool) {
    app.wizard.y_away = Some(away);
    if app.run("machine.set", json!({ "origin": origin_for(away) })).is_some() {
        app.view = None;
    }
}

pub(crate) fn answer_right(app: &mut LaserFoundryApp, moved_right: bool) {
    app.wizard.x_right = Some(moved_right);
}

/// Copies the bed size and S max the controller reported into the machine.
pub(crate) fn apply_controller(app: &mut LaserFoundryApp, c: &Controller) {
    let mut params = Map::new();
    if let Some((w, h)) = c.bed {
        params.insert("bed_width".into(), json!(w));
        params.insert("bed_height".into(), json!(h));
    }
    if let Some(s) = c.max_s {
        params.insert("max_s".into(), json!(s));
    }
    if !params.is_empty() && app.run("machine.set", Value::Object(params)).is_some() {
        app.view = None;
    }
}

/// Names the machine and closes the wizard. Returns whether it saved.
pub(crate) fn save(app: &mut LaserFoundryApp) -> bool {
    let name: String = app.wizard.name.trim().chars().take(MAX_NAME_CHARS).collect();
    if name.is_empty() {
        app.status = Some((tr("Give the machine a name"), true));
        return false;
    }
    if app.run("machine.set", json!({ "name": name })).is_none() {
        return false;
    }
    finish(app);
    app.say(tr("Machine saved"));
    true
}

/// Shows the window when the wizard is open. Closing it with the window's own button counts as skipping.
pub(crate) fn window(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.wizard.open {
        return;
    }
    let mut open = true;
    egui::Window::new(tr("Machine setup"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| body(app, ui));
    if !open && app.wizard.open {
        finish(app);
    }
}

fn body(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.weak(tr("Not verified on real hardware yet: go slowly the first time and keep a hand near the stop button."));
    ui.separator();
    match app.wizard.step {
        Step::Connect => connect_step(app, ui),
        Step::Read => read_step(app, ui),
        Step::Direction => direction_step(app, ui),
        Step::Save => save_step(app, ui),
    }
}

fn connect_step(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.heading(tr("1. Connect"));
    if app.platform.machine_available() {
        let connected = app.link.is_some();
        ui.add_enabled_ui(!connected, |ui| {
            ui.radio_value(&mut app.target, Target::Simulator, tr("No laser yet (simulator)"));
            for port in app.ports.clone() {
                ui.radio_value(&mut app.target, Target::Serial(port.clone()), port);
            }
            if ui.small_button(tr("Rescan serial ports")).clicked() {
                app.ports = app.platform.serial_ports();
            }
        });
        ui.horizontal(|ui| {
            if connected {
                ui.label(tr("Connected"));
                if ui.button(tr("Disconnect")).clicked() {
                    app.disconnect();
                }
            } else if ui.button(tr("Connect")).clicked() {
                app.connect();
            }
        });
    } else {
        ui.label(tr("Machine connections are not available in this build."));
    }
    if ui.button(tr("Skip to naming")).clicked() {
        app.wizard.step = Step::Save;
    }
    let connected = app.link.is_some();
    nav(app, ui, None, Some((Step::Read, connected)));
}

fn read_step(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.heading(tr("2. Read the controller"));
    if app.link.is_none() {
        ui.label(tr("Not connected: go back and connect, or skip on."));
        nav(app, ui, Some(Step::Connect), Some((Step::Direction, true)));
        return;
    }
    let busy = app.job_active();
    if ui.add_enabled(!busy, egui::Button::new(tr("Read settings ($$)"))).clicked() {
        app.link_do(|l| l.request_settings());
    }
    let settings = app.snap.as_ref().map(|s| s.settings.clone()).unwrap_or_default();
    if settings.is_empty() {
        ui.weak(tr("Nothing read yet."));
    } else {
        let c = read_controller(&settings);
        egui::Grid::new("wizard_controller").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label(tr("Travel ($130, $131)"));
            ui.label(c.bed.map_or("—".to_string(), |(w, h)| format!("{w} × {h} mm")));
            ui.end_row();
            ui.label(tr("Max S ($30)"));
            ui.label(shown(c.max_s));
            ui.end_row();
            ui.label(tr("Homing ($22)"));
            ui.label(c.homing.map_or("—".to_string(), |on| if on { tr("on") } else { tr("off") }));
            ui.end_row();
            ui.label(tr("Laser mode ($32)"));
            ui.label(c.laser_mode.map_or("—".to_string(), |on| if on { tr("on") } else { tr("off") }));
            ui.end_row();
        });
        if c.laser_mode == Some(false) {
            ui.colored_label(app.tokens.error, tr("Laser mode is off: the beam stays on during travel moves. Turn it on before a job."));
            if ui.add_enabled(!busy, egui::Button::new(tr("Turn on laser mode ($32=1)"))).clicked() {
                app.link_do(|l| l.write_setting(32, 1.0));
            }
        }
        if (c.bed.is_some() || c.max_s.is_some()) && ui.button(tr("Use the travel and S max for this machine")).clicked() {
            apply_controller(app, &c);
        }
    }
    nav(app, ui, Some(Step::Connect), Some((Step::Direction, true)));
}

fn direction_step(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.heading(tr("3. Direction test (laser off)"));
    ui.label(tr("The laser stays off. Stand where you normally stand, with room around the head. Presses move it a few mm."));
    let ready = app.link.is_some() && !app.job_active();
    if app.link.is_none() {
        ui.weak(tr("Not connected: you can still answer from what you know, or skip on."));
    }
    ui.add_enabled_ui(ready, |ui| {
        ui.horizontal(|ui| {
            if ui.button(tr("Press Up")).clicked() {
                app.link_do(|l| l.jog(0.0, TEST_JOG_MM, TEST_JOG_FEED));
            }
            if ui.button(tr("Press Right")).clicked() {
                app.link_do(|l| l.jog(TEST_JOG_MM, 0.0, TEST_JOG_FEED));
            }
        });
    });
    ui.separator();
    ui.label(tr("Up: did the head move away from you?"));
    ui.horizontal(|ui| {
        if ui.selectable_label(app.wizard.y_away == Some(true), tr("Yes, away")).clicked() {
            answer_up(app, true);
        }
        if ui.selectable_label(app.wizard.y_away == Some(false), tr("No, toward me")).clicked() {
            answer_up(app, false);
        }
    });
    if app.wizard.y_away.is_some() {
        let origin = if app.session.doc.machine.origin == Origin::BottomLeft { tr("bottom-left") } else { tr("top-left") };
        ui.weak(trf!("Origin set to {origin}.", origin = origin));
    }
    ui.label(tr("Right: did the head move to the right?"));
    ui.horizontal(|ui| {
        if ui.selectable_label(app.wizard.x_right == Some(true), tr("Yes, right")).clicked() {
            answer_right(app, true);
        }
        if ui.selectable_label(app.wizard.x_right == Some(false), tr("No, to the left")).clicked() {
            answer_right(app, false);
        }
    });
    if app.wizard.x_right == Some(false) {
        ui.colored_label(app.tokens.error, tr("The X axis runs backwards. Reverse it in the controller ($3), then test again."));
        let flipped = app.snap.as_ref().and_then(|s| flip_x_direction(&s.settings));
        if let Some(value) = flipped {
            if ui.add_enabled(ready, egui::Button::new(tr("Reverse X ($3)"))).clicked() {
                app.link_do(|l| l.write_setting(3, value));
                app.wizard.x_right = None;
            }
        } else {
            ui.weak(tr("Read the controller's settings first (step 2) to change this here."));
        }
    }
    nav(app, ui, Some(Step::Read), Some((Step::Save, true)));
}

fn save_step(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.heading(tr("4. Name and save"));
    let m = &app.session.doc.machine;
    let origin = if m.origin == Origin::BottomLeft { tr("bottom-left") } else { tr("top-left") };
    ui.label(trf!("Bed {w} × {h} mm, origin {origin}", w = m.bed_width, h = m.bed_height, origin = origin));
    ui.horizontal(|ui| {
        ui.label(tr("Name"));
        ui.add(egui::TextEdit::singleline(&mut app.wizard.name).desired_width(220.0));
    });
    ui.weak(tr("The machine is kept in your project. A list of machines comes later."));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(tr("Back")).clicked() {
            app.wizard.step = Step::Direction;
        }
        if ui.button(tr("Save machine")).clicked() {
            save(app);
        }
        if ui.button(tr("Skip setup")).clicked() {
            finish(app);
        }
    });
}

/// Back, Next and Skip along the bottom of a step.
fn nav(app: &mut LaserFoundryApp, ui: &mut egui::Ui, back: Option<Step>, next: Option<(Step, bool)>) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if let Some(step) = back && ui.button(tr("Back")).clicked() {
            app.wizard.step = step;
        }
        if let Some((step, enabled)) = next && ui.add_enabled(enabled, egui::Button::new(tr("Next"))).clicked() {
            app.wizard.step = step;
        }
        if ui.button(tr("Skip setup")).clicked() {
            finish(app);
        }
    });
}

fn shown<T: Display>(value: Option<T>) -> String {
    value.map_or("—".to_string(), |v| v.to_string())
}
