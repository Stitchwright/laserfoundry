use crate::i18n::{tr, trf};
use laserfoundry_device::{JobState, Link};
use laserfoundry_doc::{Anchor, Dither, LayerMode, Origin, ShapeKind, StartFrom, TextAlign};
use serde_json::{Value, json};

use crate::menus::format_duration;

use crate::{LaserFoundryApp, Target, Tool};

pub fn toolbar(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    for (tool, label) in [
        (Tool::Select, "Select"),
        (Tool::Rectangle, "Rectangle"),
        (Tool::Ellipse, "Ellipse"),
        (Tool::Line, "Line"),
        (Tool::Polygon, "Polygon"),
        (Tool::Text, "Text"),
        (Tool::Nodes, "Nodes"),
        (Tool::Pen, "Pen"),
        (Tool::Hand, "Pan"),
    ] {
        let size = [(ui.available_width() - 4.0).max(72.0), if app.tablet_active() { 40.0 } else { 28.0 }];
        if ui.add_sized(size, egui::Button::new(tr(label)).selected(app.tool == tool)).clicked() {
            if app.tool == Tool::Pen && tool != Tool::Pen {
                // Switching away completes the path being drawn rather than losing it.
                crate::nodes_ui::finish(app, false);
            }
            app.tool = tool;
            app.node_sel = None;
        }
    }
    if app.tool == Tool::Rectangle {
        ui.add_space(6.0);
        ui.label(tr("Corner radius"));
        ui.add(egui::DragValue::new(&mut app.corner_radius).range(0.0..=500.0).speed(0.1).suffix(" mm"));
    }
    if app.tool == Tool::Polygon {
        ui.add_space(6.0);
        ui.label(tr("Sides"));
        ui.add(egui::DragValue::new(&mut app.polygon_sides).range(3..=100));
    }
}

pub fn inspector(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::CollapsingHeader::new("Cuts / Layers").default_open(true).show(ui, |ui| layers(app, ui));
        egui::CollapsingHeader::new("Selection").default_open(true).show(ui, |ui| {
            properties(app, ui);
            node_section(app, ui);
            text_properties(app, ui);
            image_properties(app, ui);
            shape_operations(app, ui);
            array_section(app, ui);
            override_section(app, ui);
        });
        egui::CollapsingHeader::new("Laser").default_open(true).show(ui, |ui| laser(app, ui));
    });
}

fn layers(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let used: Vec<u32> = app.session.doc.shapes.iter().map(|s| s.layer).collect();
    let layer_list = app.session.doc.layers.clone();
    // Chips are laid out in rows of eight, like a palette.
    for row in layer_list.chunks(8) {
        ui.horizontal(|ui| {
            for layer in row {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 24.0), egui::Sense::click());
                let color = app.tokens.layer_color(&layer.color);
                let fill = if layer.output { color } else { color.gamma_multiply(0.25) };
                ui.painter().rect_filled(rect, 3.0, fill);
                if app.active_layer == layer.id {
                    ui.painter().rect_stroke(rect.expand(2.0), 4.0, egui::Stroke::new(2.0, app.tokens.selection), egui::StrokeKind::Outside);
                }
                if used.contains(&layer.id) {
                    ui.painter().circle_filled(rect.right_top() + egui::vec2(-5.0, 5.0), 2.5, egui::Color32::BLACK);
                }
                let name = if layer.output { trf!("Layer {name}", name = layer.name) } else { trf!("Layer {name} (output off)", name = layer.name) };
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, app.active_layer == layer.id, name.clone()));
                if resp.on_hover_text(&name).clicked() {
                    app.active_layer = layer.id;
                }
            }
        });
    }
    let Some(layer) = app.session.doc.layer(app.active_layer).cloned() else { return };
    ui.add_space(4.0);
    layer_header(app, ui, &layer);
    let mut output = layer.output;
    if ui.checkbox(&mut output, tr("Output")).changed() {
        app.run("layer.set", json!({ "id": layer.id, "output": output }));
    }
    egui::Grid::new("layer_settings").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label(tr("Mode"));
        let mut mode = layer.mode;
        egui::ComboBox::from_id_salt("mode")
            .selected_text(match mode {
                LayerMode::Line => tr("Line"),
                LayerMode::Fill => tr("Fill"),
                LayerMode::FillAndLine => tr("Fill + Line"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut mode, LayerMode::Line, tr("Line"));
                ui.selectable_value(&mut mode, LayerMode::Fill, tr("Fill"));
                ui.selectable_value(&mut mode, LayerMode::FillAndLine, tr("Fill + Line"));
            });
        if mode != layer.mode {
            app.run("layer.set", json!({ "id": layer.id, "mode": mode }));
        }
        ui.end_row();

        for (label, key, value, range, suffix) in [
            ("Speed", "speed", layer.speed, 1.0..=100_000.0, " mm/min"),
            ("Max power", "max_power", layer.max_power, 0.0..=100.0, " %"),
            ("Min power", "min_power", layer.min_power, 0.0..=100.0, " %"),
            ("Interval", "interval", layer.interval, 0.005..=10.0, " mm"),
            ("Scan angle", "scan_angle", layer.scan_angle, -360.0..=360.0, "°"),
            ("Kerf offset", "kerf_offset", layer.kerf_offset, -50.0..=50.0, " mm"),
            ("Tab width", "tab_width", layer.tab_width, 0.0..=1000.0, " mm"),
            ("Lead-in", "lead_in", layer.lead_in, 0.0..=1000.0, " mm"),
            ("Lead-out", "lead_out", layer.lead_out, 0.0..=1000.0, " mm"),
            ("Overscan", "overscan", layer.overscan.unwrap_or(0.0), 0.0..=50.0, " mm"),
            ("Ramp length", "ramp_length", layer.ramp_length, 0.0..=1000.0, " mm"),
            ("Z offset", "z_offset", layer.z_offset, -200.0..=200.0, " mm"),
            ("Z per pass", "z_per_pass", layer.z_per_pass, -200.0..=200.0, " mm"),
            ("Dash cut", "perf_cut", layer.perf_cut, 0.0..=1000.0, " mm"),
            ("Dash gap", "perf_gap", layer.perf_gap, 0.0..=1000.0, " mm"),
        ] {
            ui.label(tr(label));
            if let Some(v) = number(ui, value, range, suffix) {
                app.run("layer.set", json!({ "id": layer.id, key: v }));
            }
            ui.end_row();
        }

        ui.label(tr("Tabs"));
        let mut tabs = layer.tab_count;
        let r = ui.add(egui::DragValue::new(&mut tabs).range(0..=1000));
        if committed(&r) && tabs != layer.tab_count {
            app.run("layer.set", json!({ "id": layer.id, "tab_count": tabs }));
        }
        ui.end_row();

        ui.label(tr("Passes"));
        let mut passes = layer.passes;
        let r = ui.add(egui::DragValue::new(&mut passes).range(1..=1000));
        if committed(&r) && passes != layer.passes {
            app.run("layer.set", json!({ "id": layer.id, "passes": passes }));
        }
        ui.end_row();

        ui.label(tr("Image"));
        let mut dither = layer.dither;
        egui::ComboBox::from_id_salt("dither").selected_text(tr(dither.label())).show_ui(ui, |ui| {
            for d in Dither::ALL {
                ui.selectable_value(&mut dither, d, tr(d.label()));
            }
        });
        if dither != layer.dither {
            app.run("layer.set", json!({ "id": layer.id, "dither": dither }));
        }
        ui.end_row();
    });
    let mut bidi = layer.bidirectional;
    if ui.checkbox(&mut bidi, tr("Bi-directional fill")).changed() {
        app.run("layer.set", json!({ "id": layer.id, "bidirectional": bidi }));
    }
    let mut cross = layer.crosshatch;
    if ui.checkbox(&mut cross, tr("Cross-hatch fill")).on_hover_text(tr("Fill a second time at right angles")).changed() {
        app.run("layer.set", json!({ "id": layer.id, "crosshatch": cross }));
    }
    let mut air = layer.air_assist;
    if ui.checkbox(&mut air, tr("Air assist")).changed() {
        app.run("layer.set", json!({ "id": layer.id, "air_assist": air }));
    }
    presets_section(app, ui, layer.id);
}

/// Named cut settings kept in the project: apply one to this layer, or save this layer's as one.
fn presets_section(app: &mut LaserFoundryApp, ui: &mut egui::Ui, layer: u32) {
    ui.collapsing(tr("Presets"), |ui| {
        let names: Vec<String> = app.session.doc.presets.iter().map(|p| p.name.clone()).collect();
        if names.is_empty() {
            ui.weak(tr("No presets yet. Set up this layer, name it below and save."));
        }
        if ui
            .small_button(tr("Add starter presets"))
            .on_hover_text(tr("Generic first guesses, by operation. Always run a material test grid before trusting them."))
            .clicked()
        {
            app.run("preset.add_starters", json!({}));
        }
        for name in &names {
            ui.horizontal(|ui| {
                if ui.button(name).on_hover_text(tr("Apply to this layer")).clicked() {
                    app.run("preset.apply", json!({ "name": name, "layer": layer }));
                }
                if ui.small_button("✕").on_hover_text(tr("Delete this preset")).clicked() {
                    app.run("preset.delete", json!({ "name": name }));
                }
            });
        }
        if let Some(file) = app.platform_preset_library() {
            ui.horizontal(|ui| {
                if ui.small_button(tr("Save all to my library")).on_hover_text(tr("Keep these presets for other projects")).clicked() {
                    app.save_preset_library(&file);
                }
                if ui.small_button(tr("Add from my library")).clicked() {
                    app.load_preset_library(&file);
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.preset_name).hint_text("Preset name").desired_width(120.0));
            if ui.add_enabled(!app.preset_name.trim().is_empty(), egui::Button::new(tr("Save"))).clicked()
                && app.run("preset.save", json!({ "name": app.preset_name, "layer": layer })).is_some()
            {
                app.preset_name.clear();
            }
        });
    });
}

/// Rectangles and ellipses from typed positions and sizes, for when dragging is not precise enough.
fn create_by_numbers(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.separator();
    ui.label(egui::RichText::new(tr("Create by numbers")).strong());
    egui::Grid::new("create_numbers").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for (label, slot, min) in [("X", 0, -1e6), ("Y", 1, -1e6), ("Width", 2, 0.01), ("Height", 3, 0.01)] {
            ui.label(tr(label));
            if let Some(v) = app.new_shape.get_mut(slot) {
                ui.add(egui::DragValue::new(v).range(min..=1e5).speed(0.5).suffix(" mm").max_decimals(3));
            }
            ui.end_row();
        }
    });
    let [x, y, w, h] = app.new_shape;
    let layer = app.active_layer;
    ui.horizontal(|ui| {
        if ui.button(tr("Rectangle")).clicked() {
            app.run("shape.rectangle", json!({ "x": x, "y": y, "width": w, "height": h, "layer": layer }));
        }
        if ui.button(tr("Ellipse")).clicked() {
            app.run("shape.ellipse", json!({ "cx": x + w / 2.0, "cy": y + h / 2.0, "rx": w / 2.0, "ry": h / 2.0, "layer": layer }));
        }
        if ui.button(tr("Star")).on_hover_text(tr("Five points, fitted to the width and height boxes")).clicked() {
            let r = w.min(h) / 2.0;
            app.run(
                "shape.star",
                json!({ "cx": x + w / 2.0, "cy": y + h / 2.0, "outer_radius": r, "inner_radius": r * 0.4, "points": 5, "layer": layer }),
            );
        }
    });
}

fn properties(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let sel = app.session.selection.clone();
    let Some(b) = app.session.doc.bounds_of(&sel) else {
        ui.weak(tr("Nothing selected"));
        create_by_numbers(app, ui);
        return;
    };
    ui.label(if sel.len() == 1 { tr("1 object") } else { trf!("{count} objects", count = sel.len()) });
    egui::Grid::new("props").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label(tr("X"));
        if let Some(x) = number(ui, b.x0, -1e6..=1e6, " mm") {
            app.run("edit.move_to", json!({ "x": x, "y": b.y0 }));
        }
        ui.end_row();
        ui.label(tr("Y"));
        if let Some(y) = number(ui, b.y0, -1e6..=1e6, " mm") {
            app.run("edit.move_to", json!({ "x": b.x0, "y": y }));
        }
        ui.end_row();
        ui.label(tr("Width"));
        if let Some(w) = number(ui, b.width(), 0.01..=1e5, " mm") {
            app.run("edit.resize", json!({ "width": w }));
        }
        ui.end_row();
        ui.label(tr("Height"));
        if let Some(h) = number(ui, b.height(), 0.01..=1e5, " mm") {
            app.run("edit.resize", json!({ "height": h }));
        }
        ui.end_row();
    });
    ui.horizontal(|ui| {
        if ui.button(tr("Rotate 90°")).clicked() {
            app.run("edit.rotate", json!({ "degrees": 90 }));
        }
        if ui.button(tr("Flip H")).clicked() {
            app.run("edit.flip", json!({ "axis": "horizontal" }));
        }
        if ui.button(tr("Flip V")).clicked() {
            app.run("edit.flip", json!({ "axis": "vertical" }));
        }
    });
    if ui.button(trf!("Move to layer {layer}", layer = app.active_layer)).clicked() {
        app.run("edit.set_layer", json!({ "layer": app.active_layer }));
    }
}

fn laser(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let m = app.session.doc.machine.clone();
    ui.collapsing(tr("Device"), |ui| {
        egui::Grid::new("device").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label(tr("Bed width"));
            if let Some(v) = number(ui, m.bed_width, 1.0..=100_000.0, " mm") {
                app.run("machine.set", json!({ "bed_width": v }));
                app.view = None;
            }
            ui.end_row();
            ui.label(tr("Bed height"));
            if let Some(v) = number(ui, m.bed_height, 1.0..=100_000.0, " mm") {
                app.run("machine.set", json!({ "bed_height": v }));
                app.view = None;
            }
            ui.end_row();
            ui.label(tr("Origin"));
            let mut origin = m.origin;
            egui::ComboBox::from_id_salt("origin").selected_text(if origin == Origin::BottomLeft { tr("Bottom-left") } else { tr("Top-left") }).show_ui(
                ui,
                |ui| {
                    ui.selectable_value(&mut origin, Origin::BottomLeft, tr("Bottom-left"));
                    ui.selectable_value(&mut origin, Origin::TopLeft, tr("Top-left"));
                },
            );
            if origin != m.origin {
                app.run("machine.set", json!({ "origin": origin }));
            }
            ui.end_row();
            ui.label(tr("Acceleration"));
            if let Some(v) = number(ui, m.acceleration, 0.0..=100_000.0, " mm/s²") {
                app.run("machine.set", json!({ "acceleration": v }));
            }
            ui.end_row();
            ui.label(tr("Corner deviation"));
            if let Some(v) = number(ui, m.junction_deviation, 0.0..=10.0, " mm") {
                app.run("machine.set", json!({ "junction_deviation": v }));
            }
            ui.end_row();
            ui.label(tr("Overscan"));
            let mut by_speed = m.overscan_by_speed;
            if ui.checkbox(&mut by_speed, tr("from speed")).on_hover_text(tr("Run-up and run-out sized to the layer's speed and the acceleration above")).changed() {
                app.run("machine.set", json!({ "overscan_by_speed": by_speed }));
            }
            ui.end_row();
            ui.label(tr("Max S ($30)"));
            let mut max_s = m.max_s;
            if committed(&ui.add(egui::DragValue::new(&mut max_s).range(1..=100_000))) && max_s != m.max_s {
                app.run("machine.set", json!({ "max_s": max_s }));
            }
            ui.end_row();
        });
        let mut dialect = m.dialect;
        ui.horizontal(|ui| {
            ui.label(tr("G-code"));
            egui::ComboBox::from_id_salt("dialect").selected_text(tr(dialect.label())).show_ui(ui, |ui| {
                for d in laserfoundry_doc::Dialect::ALL {
                    ui.selectable_value(&mut dialect, d, tr(d.label()));
                }
            });
        });
        if dialect != m.dialect {
            app.run("machine.set", json!({ "dialect": dialect }));
        }
        if dialect != laserfoundry_doc::Dialect::Grbl {
            ui.weak(tr("Written from the firmware's public documentation and untested on a machine. Use Frame at 0% power first. The controller-link features ($$, jog) are GRBL only."));
        }
        rotary_section(app, ui, &m);
        let mut dynamic = m.dynamic_power;
        if ui.checkbox(&mut dynamic, tr("Dynamic power (M4)")).changed() {
            app.run("machine.set", json!({ "dynamic_power": dynamic }));
        }
    });

    let mut only = app.session.job_selection_only;
    if ui.checkbox(&mut only, tr("Cut selected shapes only")).on_hover_text(tr("Generate, estimate, frame and preview just the selection")).changed()
    {
        app.run("job.set_scope", json!({ "selection_only": only }));
    }
    job_origin(app, ui);
    crate::register_ui::section(app, ui);
    cut_order(app, ui);
    test_grid_section(app, ui);
    ui.separator();
    connection(app, ui);
    job_controls(app, ui);
    if app.link.is_some() {
        jog_pad(app, ui);
        crate::settings_ui::section(app, ui);
        console(app, ui);
    }
    interrupted_job(app, ui);
    if let Some(gcode) = app.session.last_gcode.clone() {
        ui.collapsing(tr("G-code"), |ui| {
            let lines = gcode.lines().count();
            ui.weak(trf!("{lines} lines", lines = lines));
            resume_controls(app, ui, &gcode, lines);
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                // Show the head only: a raster job can be millions of lines.
                let head: String = gcode.lines().take(300).collect::<Vec<_>>().join("\n");
                ui.add(egui::Label::new(egui::RichText::new(head).monospace().small()).wrap_mode(egui::TextWrapMode::Extend));
            });
        });
    }
}

fn generate(app: &mut LaserFoundryApp) {
    if app.run("job.gcode", Value::Null).is_none() {
        return;
    }
    if let Some(v) = app.run("job.estimate", Value::Null) {
        let est = format_duration(v["seconds"].as_f64().unwrap_or(0.0));
        let lines = app.session.last_gcode.as_ref().map_or(0, |g| g.lines().count());
        app.say(trf!("{lines} lines of G-code · estimated {est}", lines = lines, est = est));
    }
}

/// A numeric field that reports a value only when the user finishes editing it, so dragging does
/// not create an undo step per frame.
fn number(ui: &mut egui::Ui, value: f64, range: std::ops::RangeInclusive<f64>, suffix: &str) -> Option<f64> {
    let mut v = value;
    let r = ui.add(egui::DragValue::new(&mut v).range(range).suffix(suffix).speed(0.1).max_decimals(3));
    (committed(&r) && (v - value).abs() > 1e-9).then_some(v)
}

pub(crate) fn committed(r: &egui::Response) -> bool {
    r.drag_stopped() || r.lost_focus() || (r.changed() && !r.dragged() && !r.has_focus())
}

/// Workspace-direction step → machine-axis step (the machine's Y axis may point up or down).
pub(crate) fn machine_delta(origin: Origin, dx: f64, dy: f64) -> (f64, f64) {
    match origin {
        Origin::TopLeft => (dx, dy),
        Origin::BottomLeft => (dx, -dy),
    }
}

fn cut_order(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let o = app.session.doc.optimize;
    ui.collapsing(tr("Cut order"), |ui| {
        for (key, label, tip, on) in [
            ("inner_first", "Inner shapes first", "Cut holes and nested shapes before the outlines around them", o.inner_first),
            ("reduce_travel", "Reduce travel", "Go to the nearest outline next instead of following the drawing order", o.reduce_travel),
            ("remove_duplicates", "Remove duplicate lines", "Skip outlines that lie exactly on top of another one", o.remove_duplicates),
            ("by_group", "Finish each group first", "Cut one group of shapes completely before starting the next", o.by_group),
        ] {
            let mut v = on;
            if ui.checkbox(&mut v, tr(label)).on_hover_text(tr(tip)).changed() {
                app.run("job.set_optimize", json!({ key: v }));
            }
        }
        let start = o.start;
        ui.horizontal(|ui| {
            ui.label(tr("Start near"));
            let mut xy = start.unwrap_or([0.0, 0.0]);
            let mut changed = false;
            for v in &mut xy {
                changed |= committed(&ui.add(egui::DragValue::new(v).speed(0.5).suffix(" mm")));
            }
            if changed {
                app.run("job.set_optimize", json!({ "start": xy }));
            }
            if start.is_some() && ui.small_button(tr("Origin")).on_hover_text(tr("Back to the bed's origin")).clicked() {
                app.run("job.set_optimize", json!({ "default_start": true }));
            }
        });
    });
}

fn rotary_section(app: &mut LaserFoundryApp, ui: &mut egui::Ui, m: &laserfoundry_doc::Machine) {
    let mut on = m.rotary.enabled;
    if ui
        .checkbox(&mut on, tr("Rotary attachment"))
        .on_hover_text(tr("Y turns the workpiece: Y distances are scaled by reference / object diameter"))
        .changed()
    {
        app.run("machine.set", json!({ "rotary_enabled": on }));
    }
    if !m.rotary.enabled {
        return;
    }
    egui::Grid::new("rotary").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label(tr("Roller / chuck diameter"));
        if let Some(v) = number(ui, m.rotary.reference_diameter, 1.0..=10_000.0, " mm") {
            app.run("machine.set", json!({ "rotary_reference_diameter": v }));
        }
        ui.end_row();
        ui.label(tr("Object diameter"));
        if let Some(v) = number(ui, m.rotary.object_diameter, 1.0..=10_000.0, " mm") {
            app.run("machine.set", json!({ "rotary_object_diameter": v }));
        }
        ui.end_row();
    });
    if ui.button(tr("Set bed height to the circumference")).on_hover_text(tr("The design's Y range becomes one turn of the object")).clicked() {
        let c = m.rotary.circumference();
        app.run("machine.set", json!({ "bed_height": c }));
        app.view = None;
    }
}

fn job_origin(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let o = app.session.doc.job_origin;
    ui.horizontal(|ui| {
        ui.label(tr("Start from"));
        let mut start_from = o.start_from;
        egui::ComboBox::from_id_salt("start_from")
            .selected_text(match start_from {
                StartFrom::Absolute => tr("Absolute coordinates"),
                StartFrom::UserOrigin => tr("User origin"),
                StartFrom::CurrentPosition => tr("Current position"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut start_from, StartFrom::Absolute, tr("Absolute coordinates"));
                ui.selectable_value(&mut start_from, StartFrom::UserOrigin, tr("User origin"));
                ui.selectable_value(&mut start_from, StartFrom::CurrentPosition, tr("Current position"));
            });
        if start_from != o.start_from {
            app.run("job.set_origin", json!({ "start_from": start_from }));
        }
    });
    if o.start_from == StartFrom::Absolute {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(tr("Job origin"));
        egui::Grid::new("anchor_grid").spacing([2.0, 2.0]).show(ui, |ui| {
            for row in Anchor::ALL.chunks(3) {
                for a in row {
                    if anchor_button(ui, o.anchor == *a, &tr(a.label())).clicked() {
                        app.run("job.set_origin", json!({ "anchor": a }));
                    }
                }
                ui.end_row();
            }
        });
    });
    ui.weak(match o.start_from {
        StartFrom::UserOrigin => tr("Jog to the spot, then Set origin; that spot is the job origin."),
        _ => tr("The head's position when the job starts becomes the job origin."),
    });
}

/// Offers to pick up a job that an earlier session (or a power cut) left unfinished.
fn interrupted_job(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let Some(pos) = app.interrupted.clone() else { return };
    ui.group(|ui| {
        ui.label(trf!("A job was interrupted. It can restart from line {line} of {total}.", line = pos.line, total = pos.total_lines));
        let same = app.session.last_gcode.as_deref().is_some_and(|g| crate::job_position::fingerprint(g) == pos.hash);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(same, egui::Button::new(tr("Use this line")))
                .on_hover_text(tr("Sets Run from line below; set the origin the way the first run had it, then Start here"))
                .clicked()
            {
                app.resume_line = pos.line;
            }
            if ui.button(tr("Forget")).clicked() {
                app.forget_interrupted();
            }
        });
        if !same {
            ui.weak(tr("Open the same design and generate its G-code to use this position."));
        }
    });
}

/// Start the job part-way through: the head goes to where it would have been, with the beam off.
fn resume_controls(app: &mut LaserFoundryApp, ui: &mut egui::Ui, gcode: &str, lines: usize) {
    app.resume_line = app.resume_line.clamp(1, lines.max(1));
    ui.horizontal(|ui| {
        ui.label(tr("Run from line"));
        ui.add(egui::DragValue::new(&mut app.resume_line).range(1..=lines.max(1)).speed(1.0));
        let enabled = app.link.is_some() && !app.job_active() && app.resume_line > 1;
        if ui
            .add_enabled(enabled, egui::Button::new(tr("Start here")))
            .on_hover_text(tr("Moves the laser to where it would be at that line with the beam off, then carries on. The origin must already be set the way the first run had it."))
            .clicked()
            && let Some(v) = app.run("job.resume", json!({ "line": app.resume_line, "include": true }))
            && let Some(text) = v["gcode"].as_str()
        {
            app.start_job(text.to_string());
        }
    });
    if let Some(l) = gcode.lines().nth(app.resume_line.saturating_sub(1)) {
        ui.weak(egui::RichText::new(l).monospace());
    }
}

fn connection(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    if !app.platform.machine_available() {
        ui.weak(tr("Machine connections are not available in this build; generate G-code and run it with the desktop app or another sender."));
        return;
    }
    let connected = app.link.is_some();
    ui.horizontal(|ui| {
        ui.label(tr("Machine"));
        let current = match &app.target {
            Target::Simulator => "Simulator".to_string(),
            Target::Serial(p) => p.clone(),
        };
        ui.add_enabled_ui(!connected, |ui| {
            egui::ComboBox::from_id_salt("target").selected_text(current).show_ui(ui, |ui| {
                ui.selectable_value(&mut app.target, Target::Simulator, tr("Simulator"));
                for p in app.ports.clone() {
                    ui.selectable_value(&mut app.target, Target::Serial(p.clone()), p);
                }
            });
            if ui.small_button("↻").on_hover_text(tr("Rescan serial ports")).clicked() {
                app.ports = app.platform.serial_ports();
            }
        });
        if connected {
            if ui.button(tr("Disconnect")).clicked() {
                app.disconnect();
            }
        } else if ui.button(tr("Connect")).clicked() {
            app.connect();
        }
    });
    ui.checkbox(&mut app.auto_reconnect, tr("Reconnect automatically"))
        .on_hover_text(tr("Retry a lost USB connection; a job is never resumed by itself"));
    if app.reconnect.is_some() {
        ui.colored_label(app.tokens.error, tr("Connection lost, reconnecting…"));
    }
    if let Some(s) = &app.snap {
        let state = s.status.as_ref().map_or("…".to_string(), |st| format!("{:?}", st.state));
        let pos = s.status.as_ref().and_then(|st| st.position).map_or(String::new(), |(x, y, _)| format!("   X {x:.2}  Y {y:.2}"));
        ui.label(egui::RichText::new(format!("{state}{pos}")).monospace());
    }
}

fn job_controls(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let connected = app.link.is_some();
    let active = app.job_active();
    let paused = app.snap.as_ref().is_some_and(|s| s.job == JobState::Paused);
    ui.horizontal(|ui| {
        if ui.button(tr("Generate")).clicked() {
            generate(app);
        }
        if ui.add_enabled(connected && !active, egui::Button::new(tr("Frame"))).clicked()
            && let Some(v) = app.run("job.frame", json!({ "power": app.frame_power }))
            && let Some(g) = v["gcode"].as_str()
        {
            app.start_job(g.to_string());
        }
        if ui.add_enabled(connected && !active, egui::Button::new(tr("Start"))).clicked()
            && let Some(v) = app.run("job.gcode", json!({ "include": true }))
            && let Some(g) = v["gcode"].as_str()
        {
            app.start_job(g.to_string());
        }
    });
    ui.horizontal(|ui| {
        if paused {
            if ui.button(tr("Resume")).clicked() {
                app.link_do(Link::resume);
            }
        } else if ui.add_enabled(active, egui::Button::new(tr("Pause"))).clicked() {
            app.link_do(Link::pause);
        }
        if ui.add_enabled(connected, egui::Button::new(tr("Stop"))).on_hover_text(tr("Soft reset: stops motion and turns the laser off")).clicked() {
            app.link_do(Link::stop);
        }
        ui.label(tr("Frame power"));
        ui.add(egui::DragValue::new(&mut app.frame_power).range(0.0..=100.0).suffix(" %"));
    });
    if let Some(p) = app.snap.as_ref().and_then(|s| s.progress)
        && p.total > 0
        && active
    {
        ui.add(egui::ProgressBar::new(p.acknowledged as f32 / p.total as f32).text(format!("{} / {}", p.acknowledged, p.total)));
    }
}

fn jog_pad(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let enabled = !app.job_active();
    let origin = app.session.doc.machine.origin;
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label(tr("Step"));
            egui::ComboBox::from_id_salt("jog_step").selected_text(format!("{} mm", app.jog_step)).show_ui(ui, |ui| {
                for s in [0.1, 1.0, 10.0, 50.0] {
                    ui.selectable_value(&mut app.jog_step, s, format!("{s} mm"));
                }
            });
            ui.label(tr("Speed"));
            ui.add(egui::DragValue::new(&mut app.jog_feed).range(10.0..=30_000.0).suffix(" mm/min"));
        });
        let mut here = app.move_head_mode;
        if ui.checkbox(&mut here, tr("Click the canvas to move the laser there")).changed() {
            app.move_head_mode = here;
        }
        let step = app.jog_step;
        let jog = |app: &mut LaserFoundryApp, wx: f64, wy: f64| {
            let (dx, dy) = machine_delta(origin, wx * step, wy * step);
            let feed = app.jog_feed;
            app.link_do(|l| l.jog(dx, dy, feed));
        };
        // Directions are in workspace terms: "up" is toward the top of the canvas.
        let rows: [[Option<(f64, f64)>; 3]; 3] =
            [[None, Some((0.0, -1.0)), None], [Some((-1.0, 0.0)), None, Some((1.0, 0.0))], [None, Some((0.0, 1.0)), None]];
        egui::Grid::new("jog_grid").spacing([2.0, 2.0]).show(ui, |ui| {
            for cells in rows {
                for cell in cells {
                    if let Some((wx, wy)) = cell {
                        if arrow_button(ui, wx, wy).clicked() {
                            jog(app, wx, wy);
                        }
                    } else {
                        ui.allocate_space(egui::vec2(34.0, 28.0));
                    }
                }
                ui.end_row();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("Home")).on_hover_text(tr("Run the homing cycle ($H)")).clicked() {
                app.link_do(Link::home);
            }
            if ui.button(tr("Unlock")).on_hover_text(tr("Clear an alarm ($X)")).clicked() {
                app.link_do(Link::unlock);
            }
            if ui.button(tr("Set origin here")).on_hover_text(tr("Make the head's position the work origin (G92)")).clicked() {
                app.link_do(Link::set_zero);
            }
            if ui.button(tr("Go to origin")).clicked() {
                app.link_do(Link::go_to_zero);
            }
        });
    });
}

fn console(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.collapsing(tr("Console"), |ui| {
        let lines = app.snap.as_ref().map(|s| s.console.clone()).unwrap_or_default();
        egui::ScrollArea::vertical().max_height(140.0).stick_to_bottom(true).show(ui, |ui| {
            for l in lines.iter().rev().take(100).rev() {
                ui.label(egui::RichText::new(l).monospace().small());
            }
        });
        let r =
            ui.add(egui::TextEdit::singleline(&mut app.console_input).hint_text("G-code or $ command, Enter to send").desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let line = std::mem::take(&mut app.console_input);
            app.link_do(|l| l.send_line(&line));
            r.request_focus();
        }
    });
}

/// A square button with a triangle pointing along `(dx, dy)`, drawn in code (the default font has
/// no arrow glyphs, and we ship no icon assets).
fn arrow_button(ui: &mut egui::Ui, dx: f64, dy: f64) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(34.0, 28.0), egui::Sense::click());
    let name = match (dx.partial_cmp(&0.0), dy.partial_cmp(&0.0)) {
        (Some(std::cmp::Ordering::Less), _) => "Jog left",
        (Some(std::cmp::Ordering::Greater), _) => "Jog right",
        (_, Some(std::cmp::Ordering::Less)) => "Jog up",
        _ => "Jog down",
    };
    let enabled = ui.is_enabled();
    let name = tr(name);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name.clone()));
    let resp = resp.on_hover_text(name);
    let visuals = ui.style().interact(&resp);
    ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
    let (fx, fy) = (dx as f32, dy as f32);
    let c = rect.center();
    let along = egui::vec2(fx, fy) * 7.0;
    let across = egui::vec2(-fy, fx) * 6.0;
    let points = vec![c + along, c - along * 0.6 + across, c - along * 0.6 - across];
    ui.painter().add(egui::Shape::convex_polygon(points, visuals.fg_stroke.color, egui::Stroke::NONE));
    resp
}

/// One cell of the 3×3 job-origin picker: a ring, filled when selected (drawn in code).
fn anchor_button(ui: &mut egui::Ui, selected: bool, name: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, selected, trf!("Job origin {name}", name = name)));
    let resp = resp.on_hover_text(name);
    let visuals = ui.style().interact(&resp);
    ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
    let color = if selected { ui.visuals().selection.stroke.color } else { visuals.fg_stroke.color };
    if selected {
        ui.painter().circle_filled(rect.center(), 5.5, color);
    } else {
        ui.painter().circle_stroke(rect.center(), 5.0, egui::Stroke::new(1.5, color));
    }
    resp
}

/// The editor for a single selected text object.
fn text_properties(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let [id] = app.session.selection.as_slice() else {
        app.text_buffer = None;
        return;
    };
    let id = *id;
    let Some((content, style)) = app.session.doc.shape(id).and_then(|s| match &s.kind {
        ShapeKind::Text { content, style, .. } => Some((content.clone(), style.clone())),
        _ => None,
    }) else {
        app.text_buffer = None;
        return;
    };
    if app.text_buffer.as_ref().is_none_or(|(buf_id, _)| *buf_id != id) {
        app.text_buffer = Some((id, content.clone()));
    }
    ui.separator();
    ui.label(egui::RichText::new(tr("Text")).strong());
    let mut commit = None;
    if let Some((_, buffer)) = app.text_buffer.as_mut() {
        let r = ui.add(egui::TextEdit::multiline(buffer).desired_rows(2).desired_width(f32::INFINITY).hint_text("Type text; click away to apply"));
        if r.lost_focus() && *buffer != content && !buffer.trim().is_empty() {
            commit = Some(json!({ "id": id, "text": buffer.clone() }));
        }
    }
    egui::Grid::new("text_style").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for (label, key, value, range) in [
            ("Height", "height", style.height, 0.1..=1000.0),
            ("Letter gap", "letter_spacing", style.letter_spacing, 0.0..=10.0),
            ("Line gap", "line_spacing", style.line_spacing, 0.0..=10.0),
        ] {
            ui.label(tr(label));
            if let Some(v) = number(ui, value, range, if key == "height" { " mm" } else { "×" }) {
                commit = Some(json!({ "id": id, key: v }));
            }
            ui.end_row();
        }
        ui.label(tr("Align"));
        let mut align = style.align;
        egui::ComboBox::from_id_salt("text_align")
            .selected_text(match align {
                TextAlign::Left => tr("Left"),
                TextAlign::Center => tr("Center"),
                TextAlign::Right => tr("Right"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut align, TextAlign::Left, tr("Left"));
                ui.selectable_value(&mut align, TextAlign::Center, tr("Center"));
                ui.selectable_value(&mut align, TextAlign::Right, tr("Right"));
            });
        if align != style.align {
            commit = Some(json!({ "id": id, "align": align }));
        }
        ui.end_row();
        ui.label(tr("Direction"));
        let mut vertical = style.vertical;
        if ui.checkbox(&mut vertical, tr("Vertical")).on_hover_text(tr("Letters stacked top to bottom, one column per line")).changed() {
            commit = Some(json!({ "id": id, "vertical": vertical }));
        }
        ui.end_row();
    });
    if let Some(params) = commit {
        if let Some(v) = app.run("text.edit", params) {
            app.note_unsupported(&v);
        }
        app.text_buffer = None;
    }
    if let Some(w) = &app.text_warning {
        ui.colored_label(app.tokens.error, w);
    }
    ui.horizontal_wrapped(|ui| {
        let name = style.font.as_deref().map_or("Built-in stroke font".to_string(), |p| match p.strip_prefix("library:") {
            Some(folder) => folder.to_string(),
            None => std::path::Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().into_owned()),
        });
        ui.label(trf!("Font: {name}", name = name));
        if ui.button(tr("Choose font…")).on_hover_text(tr("A .ttf or .otf file of yours: draws filled outlines (use a Fill layer)")).clicked()
            && let Some(path) = app.pick_open("Choose a font", &["ttf", "otf"])
            && let Some(v) = app.run("text.edit", json!({ "id": id, "font": path }))
        {
            app.note_unsupported(&v);
            app.text_buffer = None;
        }
        if style.font.is_some() && ui.button(tr("Built-in")).clicked() {
            app.run("text.edit", json!({ "id": id, "font": "" }));
            app.text_buffer = None;
        }
        if app.platform_font_library_available() {
            let mut chosen = None;
            egui::ComboBox::from_id_salt("library_font").selected_text(tr("Library fonts")).show_ui(ui, |ui| {
                let fonts = app.platform_library_fonts();
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (name, value) in &fonts {
                        if ui.button(name).clicked() {
                            chosen = Some(value.clone());
                            ui.close();
                        }
                    }
                });
                if fonts.is_empty() {
                    ui.weak(tr("None downloaded yet: View > Font library"));
                }
            });
            if let Some(font) = chosen
                && let Some(v) = app.run("text.edit", json!({ "id": id, "font": font }))
            {
                app.note_unsupported(&v);
                app.text_buffer = None;
            }
        }
    });
    if style.font.is_some() {
        ui.horizontal_wrapped(|ui| {
            let name = style.fallback_font.as_deref().map_or("none".to_string(), |p| {
                std::path::Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().into_owned())
            });
            ui.label(trf!("Fallback font: {name}", name = name));
            if ui.button(tr("Choose…")).on_hover_text(tr("Draws characters the font above has no glyph for")).clicked()
                && let Some(path) = app.pick_open("Choose a fallback font", &["ttf", "otf"])
                && let Some(v) = app.run("text.edit", json!({ "id": id, "fallback_font": path }))
            {
                app.note_unsupported(&v);
                app.text_buffer = None;
            }
            if style.fallback_font.is_some() && ui.button(tr("None")).clicked() {
                app.run("text.edit", json!({ "id": id, "fallback_font": "" }));
                app.text_buffer = None;
            }
        });
        ui.weak(tr(
            "Outline text: set its layer to Fill to engrave it solid. The shapes are saved in the project; the font is only needed to edit the text.",
        ));
    } else {
        ui.weak(tr("Single-line stroke font: engraves each stroke in one pass (Line mode)."));
    }
}

/// Boolean combine and offset for the current selection.
fn shape_operations(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let count = app.session.selection.len();
    if count == 0 {
        return;
    }
    ui.separator();
    ui.label(egui::RichText::new(tr("Shape operations")).strong());
    ui.add_enabled_ui(count >= 2, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (label, op, hint) in [
                ("Union", "union", "Weld the shapes into one outline"),
                ("Subtract", "difference", "The first selected shape minus the others"),
                ("Intersect", "intersect", "Keep only the overlap"),
                ("XOR", "xor", "Keep everything except the overlap"),
            ] {
                if ui.button(tr(label)).on_hover_text(tr(hint)).clicked() {
                    app.run("path.boolean", json!({ "op": op }));
                }
            }
        });
    });
    if ui.button(tr("Join paths")).on_hover_text(tr("Chain open lines whose ends meet into paths, closing loops")).clicked() {
        app.run("path.join", json!({}));
    }
    ui.horizontal(|ui| {
        ui.label(tr("Offset"));
        ui.add(egui::DragValue::new(&mut app.offset_distance).range(-1000.0..=1000.0).speed(0.1).suffix(" mm"));
        egui::ComboBox::from_id_salt("offset_join").selected_text(app.offset_join).width(70.0).show_ui(ui, |ui| {
            for j in ["round", "miter", "bevel"] {
                ui.selectable_value(&mut app.offset_join, j, j);
            }
        });
    });
    ui.horizontal(|ui| {
        if ui
            .button(tr("Apply offset"))
            .on_hover_text(tr("Negative values shrink closed shapes; open lines become an outline around the line"))
            .clicked()
        {
            let params = json!({ "distance": app.offset_distance, "join": app.offset_join, "keep_originals": app.offset_keep });
            app.run("path.offset", params);
        }
        ui.checkbox(&mut app.offset_keep, tr("Keep original"));
    });
}

/// Tone adjustments for a single selected image.
fn image_properties(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let [id] = app.session.selection.as_slice() else { return };
    let id = *id;
    let Some(adjust) = app.session.doc.shape(id).and_then(|s| match &s.kind {
        ShapeKind::Image { adjust, .. } => Some(*adjust),
        _ => None,
    }) else {
        return;
    };
    ui.separator();
    ui.label(egui::RichText::new(tr("Image")).strong());
    let mut commit = None;
    egui::Grid::new("image_adjust").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for (label, key, value, range, speed) in [
            ("Brightness", "brightness", adjust.brightness, -1.0..=1.0, 0.01),
            ("Contrast", "contrast", adjust.contrast, -1.0..=1.0, 0.01),
            ("Gamma", "gamma", adjust.gamma, 0.1..=5.0, 0.02),
        ] {
            ui.label(tr(label));
            let mut v = value;
            let r = ui.add(egui::DragValue::new(&mut v).range(range).speed(speed).max_decimals(2));
            if committed(&r) && (v - value).abs() > 1e-9 {
                commit = Some(json!({ "id": id, key: v }));
            }
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        let mut negative = adjust.negative;
        if ui.checkbox(&mut negative, tr("Negative")).changed() {
            commit = Some(json!({ "id": id, "negative": negative }));
        }
        if ui.add_enabled(!adjust.is_identity(), egui::Button::new(tr("Reset"))).clicked() {
            commit = Some(json!({ "id": id, "reset": true }));
        }
    });
    if let Some(params) = commit {
        app.run("image.adjust", params);
    }
    crop_and_sharpen(app, ui, id);
    ui.weak(tr("The dither mode for engraving is set on the layer (Cuts / Layers)."));
}

fn crop_and_sharpen(app: &mut LaserFoundryApp, ui: &mut egui::Ui, id: u64) {
    ui.collapsing(tr("Crop and sharpen"), |ui| {
        ui.checkbox(&mut app.crop_mode, tr("Drag the edges on the canvas")).on_hover_text(tr("Escape leaves this mode"));
        egui::Grid::new("image_crop").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            for (label, slot) in ["Cut off left", "Cut off top", "Cut off right", "Cut off bottom"].into_iter().zip(app.crop_pct.iter_mut()) {
                ui.label(tr(label));
                ui.add(egui::DragValue::new(slot).range(0.0..=95.0).speed(0.5).suffix(" %").max_decimals(1));
                ui.end_row();
            }
        });
        let [left, top, right, bottom] = app.crop_pct.map(|p| p / 100.0);
        let fits = left + right < 1.0 && top + bottom < 1.0;
        if ui.add_enabled(fits && app.crop_pct.iter().any(|p| *p > 0.0), egui::Button::new(tr("Crop"))).clicked()
            && app.run("image.crop", json!({ "id": id, "left": left, "top": top, "right": right, "bottom": bottom })).is_some()
        {
            app.crop_pct = [0.0; 4];
            app.crop_mode = false;
        }
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut app.sharpen_amount).range(0.0..=5.0).speed(0.05).max_decimals(2));
            if ui.button(tr("Sharpen")).clicked() {
                app.run("image.sharpen", json!({ "id": id, "amount": app.sharpen_amount }));
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut app.resample_scale).range(0.05..=4.0).speed(0.01).max_decimals(2).suffix("×"));
            if ui.button(tr("Resample")).on_hover_text(tr("Change the number of pixels (not the size in mm)")).clicked() {
                app.run("image.resample", json!({ "id": id, "scale": app.resample_scale }));
            }
        });
        ui.weak(tr("These change the stored picture (Undo brings it back)."));
    });
    ui.collapsing(tr("Trace to vectors"), |ui| {
        ui.horizontal(|ui| {
            ui.label(tr("Threshold"));
            ui.add(egui::DragValue::new(&mut app.trace_threshold).range(1.0..=254.0).speed(1.0));
        });
        ui.checkbox(&mut app.trace_smooth, tr("Smooth curves")).on_hover_text(tr("Turn gentle bends into curves; sharp corners stay sharp"));
        ui.horizontal(|ui| {
            if ui.button(tr("Trace")).on_hover_text(tr("Add outlines that follow the dark areas, on top of the picture")).clicked() {
                app.run("image.trace", json!({ "id": id, "threshold": app.trace_threshold, "smooth": app.trace_smooth }));
            }
            if ui.button(tr("Trace and remove picture")).clicked() {
                app.run("image.trace", json!({ "id": id, "threshold": app.trace_threshold, "keep_image": false, "smooth": app.trace_smooth }));
            }
        });
    });
}

/// Grid and circular arrays of the current selection.
fn array_section(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    if app.session.selection.is_empty() {
        return;
    }
    ui.separator();
    ui.label(egui::RichText::new(tr("Array")).strong());
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut app.array_columns).range(1..=200).prefix("cols "));
        ui.add(egui::DragValue::new(&mut app.array_rows).range(1..=200).prefix("rows "));
        ui.add(egui::DragValue::new(&mut app.array_gap).range(-1000.0..=1000.0).speed(0.1).prefix("gap ").suffix(" mm"));
    });
    if ui.button(tr("Grid array")).on_hover_text(tr("Copy the selection into a grid; the original stays top-left")).clicked() {
        let params = json!({ "columns": app.array_columns, "rows": app.array_rows, "gap_x": app.array_gap, "gap_y": app.array_gap });
        app.run("arrange.grid", params);
    }
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut app.array_count).range(1..=1000).prefix("copies "));
        ui.add(egui::DragValue::new(&mut app.array_radius).range(0.0..=1000.0).speed(0.5).prefix("radius ").suffix(" mm"));
    });
    if ui.button(tr("Circular array")).on_hover_text(tr("Rotate copies about a centre this far below the selection")).clicked() {
        let params = json!({ "count": app.array_count, "radius": app.array_radius });
        app.run("arrange.circular", params);
    }
}

/// Per-shape cut settings that replace the layer's.
fn override_section(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let sel = app.session.selection.clone();
    let Some(first) = sel.first().and_then(|id| app.session.doc.shape(*id)) else { return };
    let layer = app.session.doc.layer(first.layer).cloned();
    let current = first.cut;
    let order = first.order;
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(tr("Cut order"));
        let mut n = order;
        let r = ui
            .add(egui::DragValue::new(&mut n).range(-1000..=1000))
            .on_hover_text(tr("Lower numbers are cut first within the layer; 0 is the default"));
        if committed(&r) && n != order {
            app.run("shape.set_order", json!({ "order": n }));
        }
    });
    ui.horizontal(|ui| {
        let mut on = current.is_some();
        if ui
            .checkbox(&mut on, tr("Own cut settings"))
            .on_hover_text(tr("Use these settings for the selected shapes instead of their layer's"))
            .changed()
        {
            if on {
                // Start from the layer's values so switching it on changes nothing until edited.
                let (speed, power) = layer.as_ref().map_or((1000.0, 50.0), |l| (l.speed, l.max_power));
                app.run("shape.cut_override", json!({ "speed": speed, "max_power": power }));
            } else {
                app.run("shape.cut_override", json!({ "clear": true }));
            }
        }
    });
    let Some(c) = current else { return };
    let (speed, power, passes) = (c.speed.unwrap_or(0.0), c.max_power.unwrap_or(0.0), c.passes.unwrap_or(1));
    egui::Grid::new("override_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label(tr("Speed"));
        if let Some(v) = number(ui, speed, 1.0..=100_000.0, " mm/min") {
            app.run("shape.cut_override", json!({ "speed": v }));
        }
        ui.end_row();
        ui.label(tr("Power"));
        if let Some(v) = number(ui, power, 0.0..=100.0, " %") {
            app.run("shape.cut_override", json!({ "max_power": v }));
        }
        ui.end_row();
        ui.label(tr("Passes"));
        let mut n = passes;
        let r = ui.add(egui::DragValue::new(&mut n).range(1..=1000));
        if committed(&r) && n != passes {
            app.run("shape.cut_override", json!({ "passes": n }));
        }
        ui.end_row();
    });
}

/// A speed × power matrix of squares for finding settings on a new material.
fn test_grid_section(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.collapsing(tr("Material test grid"), |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut app.grid_cols).range(1..=20).prefix("speeds "));
            ui.add(egui::DragValue::new(&mut app.grid_rows).range(1..=20).prefix("powers "));
            ui.add(egui::DragValue::new(&mut app.grid_size).range(1.0..=100.0).suffix(" mm"));
        });
        ui.horizontal(|ui| {
            ui.label(tr("Speed"));
            ui.add(egui::DragValue::new(&mut app.grid_speed.0).range(1.0..=100_000.0));
            ui.label(tr("to"));
            ui.add(egui::DragValue::new(&mut app.grid_speed.1).range(1.0..=100_000.0).suffix(" mm/min"));
        });
        ui.horizontal(|ui| {
            ui.label(tr("Power"));
            ui.add(egui::DragValue::new(&mut app.grid_power.0).range(0.0..=100.0));
            ui.label(tr("to"));
            ui.add(egui::DragValue::new(&mut app.grid_power.1).range(0.0..=100.0).suffix(" %"));
        });
        if ui.button(tr("Create on the active layer")).on_hover_text(tr("Squares use the active layer's mode; labels go on the next layer")).clicked()
        {
            let params = json!({
                "cols": app.grid_cols, "rows": app.grid_rows, "size": app.grid_size, "layer": app.active_layer,
                "speed_min": app.grid_speed.0, "speed_max": app.grid_speed.1, "power_min": app.grid_power.0, "power_max": app.grid_power.1,
            });
            app.run("generate.test_grid", params);
        }
    });
}

/// Name, and position in the cut order (layers are cut top to bottom of this list).
fn layer_header(app: &mut LaserFoundryApp, ui: &mut egui::Ui, layer: &laserfoundry_doc::Layer) {
    let position = app.session.doc.layers.iter().position(|l| l.id == layer.id).unwrap_or(0);
    let last = app.session.doc.layers.len().saturating_sub(1);
    ui.horizontal(|ui| {
        let mut name = layer.name.clone();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(110.0));
        if r.lost_focus() && name != layer.name && !name.trim().is_empty() {
            app.run("layer.set", json!({ "id": layer.id, "name": name }));
        }
        ui.label(trf!("#{position} of {total}", position = position + 1, total = last + 1));
        if ui.add_enabled(position > 0, egui::Button::new(tr("Earlier"))).on_hover_text(tr("Cut this layer sooner")).clicked() {
            app.run("layer.move", json!({ "id": layer.id, "index": position - 1 }));
        }
        if ui.add_enabled(position < last, egui::Button::new(tr("Later"))).on_hover_text(tr("Cut this layer later")).clicked() {
            app.run("layer.move", json!({ "id": layer.id, "index": position + 1 }));
        }
    });
}

/// Buttons for the selected node while the Nodes tool is active.
fn node_section(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    if app.tool != Tool::Nodes {
        return;
    }
    ui.separator();
    ui.label(egui::RichText::new(tr("Nodes")).strong());
    let Some((shape, np)) = crate::nodes_ui::world_nodes(app) else {
        let text_selected = app.session.selection.len() == 1
            && app.session.selection.first().and_then(|id| app.session.doc.shape(*id)).is_some_and(|s| matches!(s.kind, ShapeKind::Text { .. }));
        if text_selected {
            ui.weak(tr("Text must be converted to a path before its nodes can be edited."));
            if ui.button(tr("Convert text to path")).clicked() {
                app.run("edit.convert_to_path", json!({}));
            }
        } else {
            ui.weak(tr("Select one path to edit its nodes."));
        }
        return;
    };
    ui.weak(tr("Drag nodes and handles. Double-click a line to add a node. Delete removes the selected node."));
    let Some(sel) = app.node_sel.filter(|s| s.shape == shape) else {
        ui.weak(trf!("{count} nodes. Click a node to select it.", count = np.node_count()));
        return;
    };
    let Some(node) = np.subpaths.get(sel.subpath).and_then(|s| s.nodes.get(sel.index)) else { return };
    let closed = np.subpaths.get(sel.subpath).is_some_and(|s| s.closed);
    let segment_is_curve = {
        let sp = &np.subpaths[sel.subpath];
        let next = &sp.nodes[(sel.index + 1) % sp.nodes.len()];
        laserfoundry_geom::nodes::controls(node, next).is_some()
    };
    ui.label(trf!("Node {index} of {total}", index = sel.index + 1, total = np.subpaths[sel.subpath].nodes.len()));
    ui.horizontal_wrapped(|ui| {
        let base = json!({ "id": shape, "subpath": sel.subpath });
        let with = |extra: serde_json::Value| {
            let mut v = base.clone();
            if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            v
        };
        if ui.button(tr("Delete node")).clicked() {
            crate::nodes_ui::delete_selected(app);
        }
        if ui.button(if node.smooth { tr("Make corner") } else { tr("Make smooth") }).clicked() {
            app.run("node.set_smooth", with(json!({ "index": sel.index, "smooth": !node.smooth })));
        }
        if ui.button(if segment_is_curve { tr("Next segment: line") } else { tr("Next segment: curve") }).clicked() {
            app.run("node.set_segment", with(json!({ "segment": sel.index, "curve": !segment_is_curve })));
        }
        if ui.button(if closed { tr("Open path") } else { tr("Close path") }).clicked() {
            app.run("node.set_closed", with(json!({ "closed": !closed })));
        }
    });
}

/// Most library fonts listed.
const MAX_LIBRARY_FONTS: usize = 400;

/// The downloaded library fonts that can be drawn: folders under `<dir>/fonts` (the Foundry fonts) and
/// `<dir>/inkstitch` holding `font.json` with `glyphs.stitchfoundry` or `ltr.svg`, by the name in
/// `font.json` (else the folder's), sorted.
pub(crate) fn library_fonts(dir: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let mut fonts: Vec<(String, std::path::PathBuf)> = Vec::new();
    for group in ["fonts", "inkstitch"] {
        let Ok(entries) = std::fs::read_dir(dir.join(group)) else { continue };
        let found = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("font.json").is_file() && (p.join("glyphs.stitchfoundry").is_file() || p.join("ltr.svg").is_file()));
        for p in found.take(MAX_LIBRARY_FONTS.saturating_sub(fonts.len())) {
            let folder = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let named = std::fs::read_to_string(p.join("font.json"))
                .ok()
                .filter(|t| t.len() < 16 * 1024 * 1024)
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(|n| n.chars().take(60).collect::<String>()))
                .filter(|n| !n.is_empty());
            fonts.push((named.unwrap_or(folder), p));
        }
    }
    fonts.sort();
    fonts
}
