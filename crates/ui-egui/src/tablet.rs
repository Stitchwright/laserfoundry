//! Tablet mode: the interface made usable with fingers. When it is on, the whole interface is drawn
//! larger with roomier controls, grabbing handles and shapes tolerates a fingertip, two fingers pan and
//! zoom the canvas (see `canvas::navigate`), a Pan tool drags the view with one finger, an action bar
//! carries what a keyboard would (Undo, Redo, Delete, Duplicate, Fit and a sticky "Shift"), and the
//! inspector can be folded away to give the canvas the room.
//!
//! The mode is Auto by default: it switches on the first time a touch is seen.

use crate::i18n::tr;
use crate::{LaserFoundryApp, Tool};

/// Interface scale while the mode is on.
pub const UI_ZOOM: f32 = 1.3;
/// How close (screen points) a pointer must be to a shape, handle or node to grab it.
pub const HIT_PX_MOUSE: f32 = 6.0;
pub const HIT_PX_TOUCH: f32 = 16.0;
/// Below this many points of width, the inspector starts folded away in tablet mode.
pub const NARROW_SCREEN: f32 = 1100.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabletMode {
    /// On once a touch has been seen.
    #[default]
    Auto,
    On,
    Off,
}

impl TabletMode {
    pub const ALL: [TabletMode; 3] = [Self::Auto, Self::On, Self::Off];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic (on when touched)",
            Self::On => "On",
            Self::Off => "Off",
        }
    }
}

impl LaserFoundryApp {
    pub(crate) fn tablet_active(&self) -> bool {
        match self.tablet {
            TabletMode::On => true,
            TabletMode::Off => false,
            TabletMode::Auto => self.touch_seen,
        }
    }

    /// Grab tolerance in screen points.
    pub(crate) fn hit_px(&self) -> f32 {
        if self.tablet_active() { HIT_PX_TOUCH } else { HIT_PX_MOUSE }
    }
}

/// Whether the inspector starts folded away on a screen this many points wide.
pub fn starts_folded(width: f32) -> bool {
    width < NARROW_SCREEN
}

/// Run at the start of every frame: notices touch, applies or removes the larger interface, and
/// makes the sticky Shift count as a held key.
pub(crate) fn begin_frame(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.touch_seen && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Touch { .. }))) {
        app.touch_seen = true;
    }
    let active = app.tablet_active();
    if app.applied_tablet != Some(active) {
        app.applied_tablet = Some(active);
        ctx.set_zoom_factor(if active { UI_ZOOM } else { 1.0 });
        // The canvas changes size over the next frames (egui applies the new scale a frame later): fit the bed again once it settles.
        app.refit_frames = 3;
        let roomy = egui::Spacing::default();
        ctx.all_styles_mut(|style| {
            if active {
                style.spacing.interact_size.y = 34.0;
                style.spacing.button_padding = egui::vec2(10.0, 6.0);
                style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            } else {
                style.spacing.interact_size = roomy.interact_size;
                style.spacing.button_padding = roomy.button_padding;
                style.spacing.item_spacing = roomy.item_spacing;
            }
        });
        // A narrow screen starts with the inspector folded away, once; the person can open it again.
        if active && !std::mem::replace(&mut app.inspector_decided, true) && starts_folded(ctx.content_rect().width()) {
            app.show_inspector = false;
        }
    }
    if app.refit_frames > 0 {
        app.refit_frames -= 1;
        app.view = None;
    }
    if !active {
        app.sticky_shift = false;
    }
    if app.sticky_shift {
        ctx.input_mut(|i| i.modifiers.shift = true);
    }
}

/// The row of buttons a keyboard would otherwise provide; shown only in tablet mode.
pub(crate) fn action_bar(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    egui::Panel::top("tablet_bar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let big = |text: String| egui::Button::new(text).min_size(egui::vec2(56.0, 36.0));
            let undo = ui.add(big(tr("Undo")));
            note(app, "Undo", &undo);
            if undo.clicked() {
                app.run("edit.undo", serde_json::Value::Null);
            }
            let redo = ui.add(big(tr("Redo")));
            note(app, "Redo", &redo);
            if redo.clicked() {
                app.run("edit.redo", serde_json::Value::Null);
            }
            let selected = !app.session.selection.is_empty();
            let delete = ui.add_enabled(selected, big(tr("Delete")));
            note(app, "Delete", &delete);
            if delete.clicked() {
                app.run("edit.delete", serde_json::Value::Null);
            }
            let duplicate = ui.add_enabled(selected, big(tr("Duplicate")));
            note(app, "Duplicate", &duplicate);
            if duplicate.clicked() {
                app.run("edit.duplicate", serde_json::Value::Null);
            }
            let fit = ui.add(big(tr("Fit"))).on_hover_text(tr("Fit the bed to the window"));
            note(app, "Fit", &fit);
            if fit.clicked() {
                app.view = None;
            }
            ui.separator();
            let pan = app.tool == Tool::Hand;
            let pan_button =
                ui.add(big(tr("Pan")).selected(pan)).on_hover_text(tr("Drag the view with one finger (two fingers always pan and zoom)"));
            note(app, "Pan", &pan_button);
            if pan_button.clicked() {
                app.tool = if pan { Tool::Select } else { Tool::Hand };
            }
            let shift = ui
                .add(big(tr("Shift")).selected(app.sticky_shift))
                .on_hover_text(tr("Like holding Shift: keeps a move on one axis and a shape square, adds to the selection"));
            note(app, "Shift", &shift);
            if shift.clicked() {
                app.sticky_shift = !app.sticky_shift;
            }
            let panel = ui.add(big(tr("Panel")).selected(app.show_inspector)).on_hover_text(tr("Show or hide the inspector"));
            note(app, "Panel", &panel);
            if panel.clicked() {
                app.show_inspector = !app.show_inspector;
            }
        });
    });
}

/// Remembers where a canvas context menu item was drawn, for the frame tests.
#[cfg(test)]
pub(crate) fn note_menu(app: &mut LaserFoundryApp, name: &'static str, response: &egui::Response) {
    app.button_rects.insert(name, response.rect);
}

#[cfg(not(test))]
pub(crate) fn note_menu(_app: &mut LaserFoundryApp, _name: &'static str, _response: &egui::Response) {}

/// Remembers where a button was drawn, for the frame tests (which click it).
#[cfg(test)]
fn note(app: &mut LaserFoundryApp, name: &'static str, response: &egui::Response) {
    app.button_rects.insert(name, response.rect);
}

#[cfg(not(test))]
fn note(_app: &mut LaserFoundryApp, _name: &'static str, _response: &egui::Response) {}
