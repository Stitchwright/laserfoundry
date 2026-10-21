//! The undo history window: every kept step by name, the current point marked, and a click on any
//! row goes there by undoing or redoing (`edit.history_goto`).

use serde_json::json;

use crate::LaserFoundryApp;
use crate::i18n::tr;

/// Shows the window when `history_open` is set.
pub(crate) fn window(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.history_open {
        return;
    }
    let mut open = true;
    let mut target = None;
    egui::Window::new(tr("Undo History")).open(&mut open).collapsible(false).default_width(240.0).show(ctx, |ui| {
        let (steps, position) = app.session.history();
        ui.weak(tr("Click a step to go back or forward to it."));
        egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
            if ui.selectable_label(position == 0, tr("Start")).clicked() {
                target = Some(0);
            }
            for (i, label) in steps.iter().enumerate() {
                // Steps after the current point are the ones Redo would bring back.
                let text = egui::RichText::new(tr(label));
                let text = if i < position { text } else { text.weak() };
                if ui.selectable_label(i + 1 == position, text).clicked() {
                    target = Some(i + 1);
                }
            }
        });
    });
    if let Some(position) = target {
        app.run("edit.history_goto", json!({ "position": position }));
    }
    if !open {
        app.history_open = false;
    }
}
