//! Print-and-cut / camera alignment: mark two spots in the design, tell the program where those
//! spots really are (jog the laser onto them, or click them on a photo of the bed), and the design
//! is moved and turned to fit.

use crate::i18n::{tr, trf};
use egui::{Align2, FontId, Stroke, vec2};
use laserfoundry_geom::Point;
use serde_json::json;

use crate::LaserFoundryApp;
use crate::view::View;

/// Which mark the next canvas click sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    pub actual: bool,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Marks {
    pub design: [Option<Point>; 2],
    pub actual: [Option<Point>; 2],
    pub pick: Option<Pick>,
    pub free_scale: bool,
}

impl Marks {
    /// All four marks, when they are all set.
    pub fn complete(&self) -> Option<([Point; 2], [Point; 2])> {
        match (self.design, self.actual) {
            ([Some(d0), Some(d1)], [Some(a0), Some(a1)]) => Some(([d0, d1], [a0, a1])),
            _ => None,
        }
    }

    fn set(&mut self, pick: Pick, p: Point) {
        let slot = if pick.actual { &mut self.actual } else { &mut self.design };
        if let Some(s) = slot.get_mut(pick.index) {
            *s = Some(p);
        }
    }
}

pub fn section(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.collapsing(tr("Align to marks (print and cut)"), |ui| {
        ui.weak(tr("1. Mark two spots in the design. 2. Say where they really are: jog the laser onto each printed mark, or click it on a photo of the bed. 3. Align."));
        for actual in [false, true] {
            ui.label(egui::RichText::new(if actual { tr("Where they really are") } else { tr("Marks in the design") }).strong());
            for index in 0..2 {
                mark_row(app, ui, actual, index);
            }
            if !actual {
                let two = app.session.selection.len() == 2;
                if ui.add_enabled(two, egui::Button::new(tr("Use the centres of the 2 selected shapes"))).clicked() {
                    let centres: Vec<Point> = app.session.selection.iter().filter_map(|id| app.session.doc.bounds_of(&[*id])).map(|b| b.center()).collect();
                    if let [a, b] = centres[..] {
                        app.marks.design = [Some(a), Some(b)];
                    }
                }
            }
        }
        ui.checkbox(&mut app.marks.free_scale, tr("Also correct the size (printer scaling)"));
        ui.horizontal(|ui| {
            if ui.add_enabled(app.marks.complete().is_some(), egui::Button::new(tr("Align design to marks"))).clicked()
                && let Some((design, actual)) = app.marks.complete()
            {
                let params = json!({
                    "design": design.map(|p| [p.x, p.y]),
                    "actual": actual.map(|p| [p.x, p.y]),
                    "lock_scale": !app.marks.free_scale,
                });
                if let Some(v) = app.run("align.two_point", params) {
                    app.say(trf!(
                        "Aligned {moved} shapes: turned {turned}°, size x{scale}",
                        moved = v["moved"],
                        turned = format!("{:.2}", v["rotation_deg"].as_f64().unwrap_or(0.0)),
                        scale = format!("{:.4}", v["scale"].as_f64().unwrap_or(1.0))
                    ));
                    app.marks = Marks { free_scale: app.marks.free_scale, ..Marks::default() };
                }
            }
            if ui.button(tr("Clear marks")).clicked() {
                app.marks = Marks { free_scale: app.marks.free_scale, ..Marks::default() };
            }
        });
    });
}

fn mark_row(app: &mut LaserFoundryApp, ui: &mut egui::Ui, actual: bool, index: usize) {
    let current = if actual { app.marks.actual } else { app.marks.design }.get(index).copied().flatten();
    ui.horizontal(|ui| {
        ui.label(trf!("Mark {number}", number = index + 1));
        ui.monospace(current.map_or(tr("not set"), |p| format!("{:.2}, {:.2}", p.x, p.y)));
        let picking = app.marks.pick == Some(Pick { actual, index });
        if ui.selectable_label(picking, "Click on canvas").clicked() {
            app.marks.pick = if picking { None } else { Some(Pick { actual, index }) };
        }
        if actual {
            let head = crate::head_ui::head_world(app);
            if ui
                .add_enabled(head.is_some(), egui::Button::new(tr("From laser")))
                .on_hover_text(tr("Use where the laser head is now (needs a connection and absolute coordinates)"))
                .clicked()
                && let Some(p) = head
            {
                app.marks.set(Pick { actual, index }, p);
            }
        }
    });
}

/// A canvas click while a mark is being picked. Returns whether the click was used.
pub fn click(app: &mut LaserFoundryApp, at: Point) -> bool {
    let Some(pick) = app.marks.pick else { return false };
    if at.x.is_finite() && at.y.is_finite() {
        app.marks.set(pick, at);
        app.marks.pick = None;
    }
    true
}

pub fn draw(app: &LaserFoundryApp, painter: &egui::Painter, view: &View) {
    let tokens = &app.tokens;
    for (actual, marks) in [(false, &app.marks.design), (true, &app.marks.actual)] {
        for (i, mark) in marks.iter().enumerate() {
            let Some(p) = mark else { continue };
            let at = view.to_screen(*p);
            let color = if actual { tokens.error } else { tokens.selection };
            let arm = 8.0;
            painter.line_segment([at - vec2(arm, 0.0), at + vec2(arm, 0.0)], Stroke::new(1.5, color));
            painter.line_segment([at - vec2(0.0, arm), at + vec2(0.0, arm)], Stroke::new(1.5, color));
            painter.circle_stroke(at, 5.0, Stroke::new(1.0, color));
            painter.text(
                at + vec2(8.0, -8.0),
                Align2::LEFT_BOTTOM,
                format!("{}{}", if actual { "A" } else { "D" }, i + 1),
                FontId::monospace(11.0),
                color,
            );
        }
    }
}
