//! The controller settings editor: the `$$` table the machine reports, editable in place, plus
//! checks of those values against the document's machine setup.

use crate::i18n::{tr, trf};
use std::collections::BTreeMap;

use laserfoundry_device::settings::{Kind, describe};
use laserfoundry_doc::Machine;
use serde_json::json;

use crate::LaserFoundryApp;

/// Differences between what the controller reports and what the document assumes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Advice {
    /// `$32=0`: without laser mode GRBL does not scale power during moves and `M4` is not dynamic.
    LaserModeOff,
    /// `$30` differs from the document's S max, so power percentages would come out wrong.
    SMaxDiffers(u32),
    /// `$120`/`$121` (accelerations) differ from what time estimates use.
    AccelDiffers(f64),
    /// `$130`/`$131` differ from the document's bed size.
    BedDiffers(f64, f64),
}

pub fn advice(settings: &BTreeMap<u32, String>, machine: &Machine) -> Vec<Advice> {
    let num = |id: u32| settings.get(&id).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
    let mut out = Vec::new();
    if num(32).is_some_and(|v| v == 0.0) {
        out.push(Advice::LaserModeOff);
    }
    if let Some(s) = num(30)
        && s >= 1.0
        && s <= f64::from(u32::MAX)
        && (s - f64::from(machine.max_s)).abs() > 0.5
    {
        out.push(Advice::SMaxDiffers(s.round() as u32));
    }
    if let (Some(x), Some(y)) = (num(120), num(121)) {
        let slower = x.min(y);
        if slower >= 1.0 && (slower - machine.acceleration).abs() > 0.5 {
            out.push(Advice::AccelDiffers(slower));
        }
    }
    if let (Some(w), Some(h)) = (num(130), num(131))
        && w >= 1.0
        && h >= 1.0
        && ((w - machine.bed_width).abs() > 0.05 || (h - machine.bed_height).abs() > 0.05)
    {
        out.push(Advice::BedDiffers(w, h));
    }
    out
}

pub fn section(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.collapsing(tr("Controller settings ($$)"), |ui| {
        let busy = app.job_active();
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new(tr("Read from machine"))).clicked() {
                app.link_do(|l| l.request_settings());
            }
            if busy {
                ui.weak(tr("not while a job runs"));
            }
        });
        let settings = app.snap.as_ref().map(|s| s.settings.clone()).unwrap_or_default();
        if settings.is_empty() {
            ui.weak(tr("Press the button to read the controller's settings."));
            return;
        }
        for a in advice(&settings, &app.session.doc.machine) {
            advice_row(app, ui, a, busy);
        }
        let mut write = None;
        egui::Grid::new("controller_settings").num_columns(3).spacing([8.0, 3.0]).show(ui, |ui| {
            for (id, value) in &settings {
                let info = describe(*id);
                ui.label(format!("${id}")).on_hover_text(info.map_or(tr("Not a documented GRBL 1.1 setting"), |i| tr(i.name)));
                ui.label(tr(info.map_or("(unknown)", |i| i.name)));
                ui.add_enabled_ui(!busy, |ui| {
                    if let Some(v) = edit_value(ui, info.map_or(Kind::Number, |i| i.kind), value, info.map_or("", |i| i.unit)) {
                        write = Some((*id, v));
                    }
                });
                ui.end_row();
            }
        });
        if let Some((id, v)) = write {
            app.link_do(|l| l.write_setting(id, v));
        }
    });
}

fn advice_row(app: &mut LaserFoundryApp, ui: &mut egui::Ui, a: Advice, busy: bool) {
    ui.horizontal_wrapped(|ui| match a {
        Advice::LaserModeOff => {
            ui.colored_label(app.tokens.error, tr("Laser mode ($32) is off."));
            if ui.add_enabled(!busy, egui::Button::new(tr("Turn on"))).clicked() {
                app.link_do(|l| l.write_setting(32, 1.0));
            }
        }
        Advice::SMaxDiffers(s) => {
            ui.label(trf!("Controller S max is {s}; the document uses {used}.", s = s, used = app.session.doc.machine.max_s));
            if ui.button(tr("Use controller's")).clicked() {
                app.run("machine.set", json!({ "max_s": s }));
            }
        }
        Advice::AccelDiffers(a) => {
            ui.label(trf!("Controller acceleration is {a} mm/s²; time estimates use {used}.", a = a, used = app.session.doc.machine.acceleration));
            if ui.button(tr("Use controller's")).clicked() {
                app.run("machine.set", json!({ "acceleration": a }));
            }
        }
        Advice::BedDiffers(w, h) => {
            ui.label(trf!("Controller travel is {w} × {h} mm.", w = w, h = h));
            if ui.button(tr("Use as bed size")).clicked() {
                app.run("machine.set", json!({ "bed_width": w, "bed_height": h }));
                app.view = None;
            }
        }
    });
}

/// An editor for one value; returns the new value once the person finishes changing it.
fn edit_value(ui: &mut egui::Ui, kind: Kind, value: &str, unit: &str) -> Option<f64> {
    let current = value.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0);
    match kind {
        Kind::Flag => {
            let mut on = current != 0.0;
            ui.checkbox(&mut on, "").changed().then(|| f64::from(u8::from(on)))
        }
        Kind::Mask | Kind::Number => {
            let mut v = current;
            let mut drag = egui::DragValue::new(&mut v).speed(0.1).max_decimals(4).range(0.0..=1.0e7);
            if kind == Kind::Mask {
                drag = drag.speed(1.0).max_decimals(0);
            } else if !unit.is_empty() && unit != "mask" {
                drag = drag.suffix(format!(" {unit}"));
            }
            let r = ui.add(drag);
            (crate::panels::committed(&r) && (v - current).abs() > 1e-9).then_some(v)
        }
    }
}
