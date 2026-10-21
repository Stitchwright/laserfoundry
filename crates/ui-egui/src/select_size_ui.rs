//! The Select by Size window: limits on the width and height of shapes, then Select (`edit.select_by_size`).

use serde_json::{Map, Value};

use crate::LaserFoundryApp;
use crate::i18n::{tr, trf};

/// The limits being typed. A limit is used only while its box is ticked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SizeFilter {
    /// min width, max width, min height, max height: (used, mm).
    pub limits: [(bool, f64); 4],
    pub in_selection: bool,
}

const NAMES: [(&str, &str); 4] =
    [("min_width", "Width at least"), ("max_width", "Width at most"), ("min_height", "Height at least"), ("max_height", "Height at most")];

impl SizeFilter {
    /// The command parameters for the ticked limits.
    pub fn params(&self) -> Value {
        let mut m = Map::new();
        for ((key, _), (used, mm)) in NAMES.iter().zip(&self.limits) {
            if *used {
                m.insert((*key).into(), Value::from(*mm));
            }
        }
        m.insert("in_selection".into(), Value::Bool(self.in_selection));
        Value::Object(m)
    }
}

/// Shows the window when `select_size_open` is set.
pub(crate) fn window(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.select_size_open {
        return;
    }
    let mut open = true;
    let mut go = false;
    let has_selection = !app.session.selection.is_empty();
    egui::Window::new(tr("Select by Size")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        let filter = &mut app.size_filter;
        egui::Grid::new("size_filter").num_columns(2).show(ui, |ui| {
            for ((_, label), (used, mm)) in NAMES.iter().zip(filter.limits.iter_mut()) {
                ui.checkbox(used, tr(label));
                ui.add_enabled(*used, egui::DragValue::new(mm).range(0.0..=100_000.0).speed(0.5).suffix(" mm"));
                ui.end_row();
            }
        });
        ui.add_enabled(has_selection, egui::Checkbox::new(&mut filter.in_selection, tr("Only within the current selection")));
        let any = filter.limits.iter().any(|(used, _)| *used);
        go = ui.add_enabled(any, egui::Button::new(tr("Select"))).clicked();
        if !any {
            ui.weak(tr("Tick at least one limit."));
        }
    });
    if go {
        let params = app.size_filter.params();
        if let Some(result) = app.run("edit.select_by_size", params) {
            let count = result["count"].as_u64().unwrap_or(0);
            app.say(trf!("Selected {count} objects", count = count));
        }
    }
    if !open {
        app.select_size_open = false;
    }
}
