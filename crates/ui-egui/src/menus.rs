use crate::i18n::{tr, trf};
use serde_json::{Value, json};

use crate::{LaserFoundryApp, PendingAction};

pub fn menu_bar(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    egui::Panel::top("menu").show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("File"), |ui| file_menu(app, ui));
            ui.menu_button(tr("Edit"), |ui| edit_menu(app, ui));
            ui.menu_button(tr("Arrange"), |ui| arrange_menu(app, ui));
            ui.menu_button(tr("Laser"), |ui| laser_menu(app, ui));
            if app.platform.http_available() {
                ui.menu_button(tr("AI"), |ui| ai_menu(app, ui));
            }
            ui.menu_button(tr("Help"), |ui| help_menu(app, ui));
            ui.menu_button(tr("View"), |ui| {
                if ui.button(tr("Fit bed to window")).clicked() {
                    app.view = None;
                    ui.close();
                }
                ui.horizontal(|ui| {
                    ui.label(tr("Tablet mode"));
                    egui::ComboBox::from_id_salt("tablet_mode").selected_text(tr(app.tablet.label())).show_ui(ui, |ui| {
                        for mode in crate::tablet::TabletMode::ALL {
                            ui.selectable_value(&mut app.tablet, mode, tr(mode.label()));
                        }
                    });
                });
                ui.checkbox(&mut app.show_rulers, tr("Rulers"));
                ui.checkbox(&mut app.snapping.objects, tr("Snap to objects and guides"));
                ui.add_enabled_ui(app.snapping.objects, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr("Tracking lines"));
                        let angles = &mut app.snapping.angles;
                        egui::ComboBox::from_id_salt("track_angles").selected_text(tr(angles.label())).show_ui(ui, |ui| {
                            for a in crate::snap::TrackAngles::ALL {
                                ui.selectable_value(angles, a, tr(a.label()));
                            }
                        });
                    });
                });
                let mut grid_on = app.snapping.grid.is_some();
                if ui.checkbox(&mut grid_on, tr("Snap to grid")).changed() {
                    app.snapping.grid = grid_on.then_some(app.snapping.grid.unwrap_or(5.0));
                }
                if let Some(step) = app.snapping.grid.as_mut() {
                    ui.horizontal(|ui| {
                        ui.label(tr("Grid step"));
                        egui::ComboBox::from_id_salt("grid_step").selected_text(format!("{step} mm")).show_ui(ui, |ui| {
                            for s in [0.5, 1.0, 2.0, 5.0, 10.0] {
                                ui.selectable_value(step, s, format!("{s} mm"));
                            }
                        });
                    });
                }
                if ui.add_enabled(!app.guides.is_empty(), egui::Button::new(tr("Clear guides"))).clicked() {
                    app.guides.clear();
                    ui.close();
                }
                ui.separator();
                ui.checkbox(&mut app.show_preview, tr("Toolpath preview"));
                ui.checkbox(&mut app.show_travel, tr("Show travel moves"));
                ui.menu_button(tr("Theme"), |ui| {
                    for choice in crate::theme::ThemeChoice::ALL {
                        if ui.radio_value(&mut app.theme, choice, tr(choice.label())).clicked() {
                            ui.close();
                        }
                    }
                });
                {
                    ui.menu_button(tr("Language"), |ui| {
                        if let Some(name) = crate::i18n::active_name() {
                            ui.weak(if name.is_empty() { tr("Custom translation") } else { name });
                        }
                        if ui.button(tr("Load translation…")).on_hover_text(tr("A JSON catalog of your own: see the manual")).clicked() {
                            app.choose_translation();
                            ui.close();
                        }
                        if ui.button(tr("Save translation template…")).on_hover_text(tr("Every text shown so far, to translate")).clicked() {
                            app.save_translation_template();
                            ui.close();
                        }
                        if ui.button(tr("Use English")).clicked() {
                            app.use_english();
                            ui.close();
                        }
                    });
                }
                if app.platform.font_library_available() {
                    ui.menu_button(tr("Font library"), |ui| {
                        ui.add_enabled_ui(!app.font_download_running, |ui| {
                            if ui
                                .button(tr("Download the Foundry fonts"))
                                .on_hover_text(tr("About a megabyte: Block, Script, Sans, Slab and Varsity"))
                                .clicked()
                            {
                                app.download_font_library(false);
                                ui.close();
                            }
                            if app.platform.font_library_everything_available()
                                && ui
                                    .button(tr("Download everything (large)…"))
                                    .on_hover_text(tr("Adds the Ink/Stitch collection: hundreds of megabytes"))
                                    .clicked()
                            {
                                app.download_font_library(true);
                                ui.close();
                            }
                        });
                        if let Some(dir) = app.platform.font_library_dir() {
                            ui.weak(dir.display().to_string());
                        }
                    });
                }
                ui.menu_button(tr("Preview colours"), |ui| {
                    for (mode, label) in [
                        (crate::timeline_ui::PreviewColors::Layer, "By layer"),
                        (crate::timeline_ui::PreviewColors::Power, "By power"),
                        (crate::timeline_ui::PreviewColors::Speed, "By speed"),
                    ] {
                        if ui.radio_value(&mut app.preview_colors, mode, tr(label)).clicked() {
                            ui.close();
                        }
                    }
                });
            });
        });
    });
}

/// AI: the four features and the account. Each opens its own window.
fn ai_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    use crate::ai_ui::Window;
    for (label, window) in [
        ("Design from Prompt…", Window::Design),
        ("Redraw Image…", Window::Redraw),
        ("Ask About Lasering…", Window::Ask),
        ("Check My Cut…", Window::Check),
    ] {
        if ui.add_enabled(app.ai_feature_on(window), egui::Button::new(tr(label))).clicked() {
            app.open_ai(window);
            ui.close();
        }
    }
    ui.separator();
    if item(ui, "Foundry Account…", "") {
        app.open_ai(Window::Account);
    }
}

/// Help: each entry's command returns a web address, which the system browser opens.
fn help_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    for (label, id) in [("LaserFoundry on GitHub", "help.github"), ("Report a Bug…", "help.report_bug")] {
        if item(ui, label, "")
            && let Some(url) = app.run(id, Value::Null).and_then(|v| v["url"].as_str().map(str::to_string))
        {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    }
}

fn file_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    if item(ui, "New", "Ctrl+N") {
        app.request(PendingAction::New);
    }
    if item(ui, "Open…", "Ctrl+O") {
        app.request(PendingAction::Open);
    }
    recent_menu(app, ui);
    template_menu(app, ui);
    let can_save = true;
    if ui.add_enabled(can_save, egui::Button::new(tr("Save"))).clicked() {
        save(app, false);
        ui.close();
    }
    if ui.button(tr("Save As…")).clicked() {
        save(app, true);
        ui.close();
    }
    ui.separator();
    if ui.button(tr("Import SVG…")).clicked() {
        import(app, "file.import_svg", "Import SVG", &["svg"]);
        ui.close();
    }
    if ui.button(tr("Import DXF…")).clicked() {
        import(app, "file.import_dxf", "Import DXF", &["dxf"]);
        ui.close();
    }
    if ui.button(tr("Import Image…")).clicked() {
        import(app, "file.import_image", "Import image", &["png", "jpg", "jpeg", "bmp", "gif", "webp"]);
        ui.close();
    }
    ui.separator();
    if ui.button(tr("Save SVG…")).on_hover_text(tr("For software that takes SVG but not G-code")).clicked() {
        if app.platform_browser_files() {
            if let Some(v) = app.run("file.export_svg", json!({ "include": true }))
                && let Some(text) = v["svg"].as_str()
            {
                app.download("design.svg", text.as_bytes(), "image/svg+xml");
            }
        } else if let Some(path) = app.pick_save("Save SVG", "design.svg", &["svg"])
            && app.run("file.export_svg", json!({ "path": path })).is_some()
        {
            app.say(tr("SVG saved"));
        }
        ui.close();
    }
    if ui.button(tr("Save G-code…")).clicked() {
        if app.platform_browser_files() {
            if let Some(v) = app.run("job.gcode", json!({ "include": true }))
                && let Some(text) = v["gcode"].as_str()
            {
                app.download("job.gcode", text.as_bytes(), "text/plain");
            }
        } else if let Some(path) = app.pick_save("Save G-code", "job.gcode", &["gcode", "nc"])
            && app.run("file.export_gcode", json!({ "path": path })).is_some()
        {
            app.say(tr("G-code saved"));
        }
        ui.close();
    }
}

fn template_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    if app.templates_available() {
        let templates = app.templates();
        let mut chosen = None;
        ui.menu_button(tr("New from Template"), |ui| {
            if templates.is_empty() {
                ui.weak(tr("No templates yet"));
            }
            for (name, path) in &templates {
                if ui.button(name).clicked() {
                    chosen = Some(path.clone());
                    ui.close();
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut app.template_name).hint_text("Template name").desired_width(120.0));
                if ui.add_enabled(!app.template_name.trim().is_empty(), egui::Button::new(tr("Save current"))).clicked() {
                    let name = std::mem::take(&mut app.template_name);
                    app.save_template(&name);
                    ui.close();
                }
            });
        });
        if let Some(path) = chosen {
            app.request(PendingAction::FromTemplate(path));
            ui.close();
        }
    }
}

fn recent_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    if app.platform_browser_files() {
        return browser_recent_menu(app, ui);
    }
    let mut chosen = None;
    let mut clear = false;
    ui.add_enabled_ui(!app.recent.files.is_empty(), |ui| {
        ui.menu_button(tr("Open Recent"), |ui| {
            for path in &app.recent.files {
                let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                    chosen = Some(path.clone());
                    ui.close();
                }
            }
            ui.separator();
            if ui.button(tr("Clear list")).clicked() {
                clear = true;
                ui.close();
            }
        });
    });
    if clear {
        app.recent.files.clear();
        app.save_recent();
    }
    if let Some(path) = chosen {
        app.request(PendingAction::OpenPath(path));
        ui.close();
    }
}

fn edit_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let (can_undo, can_redo) = (app.session.can_undo(), app.session.can_redo());
    use crate::shortcuts::Action;
    if ui.add_enabled(can_undo, egui::Button::new(tr("Undo")).shortcut_text(app.shortcuts.text(Action::Undo))).clicked() {
        app.run("edit.undo", Value::Null);
        ui.close();
    }
    if ui.add_enabled(can_redo, egui::Button::new(tr("Redo")).shortcut_text(app.shortcuts.text(Action::Redo))).clicked() {
        app.run("edit.redo", Value::Null);
        ui.close();
    }
    if ui.button(tr("Undo History…")).clicked() {
        app.history_open = true;
        ui.close();
    }
    ui.separator();
    for (label, id, action) in [
        ("Select All", "edit.select_all", Some(Action::SelectAll)),
        ("Duplicate", "edit.duplicate", Some(Action::Duplicate)),
        ("Delete", "edit.delete", None),
    ] {
        let shortcut = action.map_or("Del".to_string(), |a| app.shortcuts.text(a));
        if ui.add_enabled(id == "edit.select_all" || !app.session.selection.is_empty(), egui::Button::new(label).shortcut_text(shortcut)).clicked() {
            app.run(id, Value::Null);
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tr("Keyboard shortcuts…")).clicked() {
        app.shortcuts_open = true;
        ui.close();
    }
}

pub(crate) fn arrange_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.add_enabled_ui(!app.session.selection.is_empty(), |ui| {
        for (label, to) in [
            ("Align Left", "left"),
            ("Align Right", "right"),
            ("Align Top", "top"),
            ("Align Bottom", "bottom"),
            ("Center Horizontally", "center_x"),
            ("Center Vertically", "center_y"),
        ] {
            if ui.button(tr(label)).clicked() {
                app.run("edit.align", json!({ "to": to }));
                ui.close();
            }
        }
        ui.add_enabled_ui(app.session.selection.len() >= 3, |ui| {
            for (label, axis) in [("Distribute Horizontally", "horizontal"), ("Distribute Vertically", "vertical")] {
                if ui.button(tr(label)).clicked() {
                    app.run("edit.distribute", json!({ "axis": axis }));
                    ui.close();
                }
            }
        });
        if ui.button(tr("Nest in the Bed")).on_hover_text(tr("Pack the selection into the bed without overlaps")).clicked() {
            crate::panels::nest_selection(app);
            ui.close();
        }
        ui.add_enabled_ui(app.session.selection.len() == 2, |ui| {
            if ui.button(tr("Text on Path")).on_hover_text(tr("Select a text and the shape to bend it along")).clicked() {
                let is_text = |id: &u64| app.session.doc.shape(*id).is_some_and(|s| matches!(s.kind, laserfoundry_doc::ShapeKind::Text { .. }));
                if let [a, b] = app.session.selection[..] {
                    let (text, path) = if is_text(&a) { (a, b) } else { (b, a) };
                    app.run("text.on_path", json!({ "text": text, "path": path }));
                }
                ui.close();
            }
        });
        ui.separator();
        ui.add_enabled_ui(app.session.selection.len() >= 2, |ui| {
            ui.menu_button(tr("Boolean"), |ui| {
                for (label, op) in [
                    ("Union (weld)", "union"),
                    ("Subtract (first minus the rest)", "difference"),
                    ("Intersect", "intersect"),
                    ("Exclusive or", "xor"),
                ] {
                    if ui.button(tr(label)).clicked() {
                        app.run("path.boolean", json!({ "op": op }));
                        ui.close();
                    }
                }
            });
        });
        ui.separator();
        if ui.add(egui::Button::new(tr("Group")).shortcut_text(app.shortcuts.text(crate::shortcuts::Action::Group))).clicked() {
            app.run("edit.group", json!({}));
            ui.close();
        }
        if ui.add(egui::Button::new(tr("Ungroup")).shortcut_text(app.shortcuts.text(crate::shortcuts::Action::Ungroup))).clicked() {
            app.run("edit.ungroup", json!({}));
            ui.close();
        }
        ui.separator();
        if ui.button(tr("Join Paths")).clicked() {
            app.run("path.join", json!({}));
            ui.close();
        }
        ui.separator();
        if ui.button(tr("Flip Horizontal")).clicked() {
            app.run("edit.flip", json!({ "axis": "horizontal" }));
            ui.close();
        }
        if ui.button(tr("Flip Vertical")).clicked() {
            app.run("edit.flip", json!({ "axis": "vertical" }));
            ui.close();
        }
        if ui.button(tr("Rotate 90°")).clicked() {
            app.run("edit.rotate", json!({ "degrees": 90 }));
            ui.close();
        }
    });
}

fn laser_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    if ui.button(tr("Generate G-code")).clicked() {
        if app.run("job.gcode", Value::Null).is_some() {
            app.say(tr("G-code generated"));
        }
        ui.close();
    }
    if ui
        .button(tr("Save machine report…"))
        .on_hover_text(tr("The controller's settings, the console and what this program assumes, as text to send with a problem"))
        .clicked()
    {
        app.save_machine_report();
        ui.close();
    }
    if ui.button(tr("Estimate time")).clicked() {
        if let Some(v) = app.run("job.estimate", Value::Null) {
            app.say(trf!("Estimated {time}", time = format_duration(v["seconds"].as_f64().unwrap_or(0.0))));
        }
        ui.close();
    }
}

fn item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    let clicked = ui.add(egui::Button::new(tr(label)).shortcut_text(shortcut)).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Saves (asking for a path when needed). Returns whether the document was saved.
pub(crate) fn save(app: &mut LaserFoundryApp, always_ask: bool) -> bool {
    if app.platform_browser_files() {
        let Some(v) = app.run("file.project_data", Value::Null) else { return false };
        let bytes = v["data_base64"].as_str().and_then(laserfoundry_doc::base64::decode).unwrap_or_default();
        let name =
            app.session.path.as_ref().and_then(|p| p.file_name()).map_or("project.laserfoundry".to_string(), |n| n.to_string_lossy().into_owned());
        app.download(&name, &bytes, "application/json");
        app.remember_project(&name, &bytes);
        return true;
    }
    let path = if always_ask || app.session.path.is_none() { app.pick_save("Save project", "project.laserfoundry", &["laserfoundry"]) } else { None };
    let params = path.map_or(Value::Null, |p| json!({ "path": p }));
    if (always_ask && params.is_null()) || (app.session.path.is_none() && params.is_null()) {
        return false;
    }
    if app.run("file.save", params).is_some() {
        app.say(tr("Saved"));
        return true;
    }
    false
}

fn import(app: &mut LaserFoundryApp, command: &str, title: &str, exts: &[&str]) {
    if app.platform_browser_files() {
        let kind = match command {
            "file.import_svg" => crate::UploadKind::Svg,
            "file.import_dxf" => crate::UploadKind::Dxf,
            _ => crate::UploadKind::Image,
        };
        app.begin_upload(kind);
        return;
    }
    if let Some(path) = app.pick_open(title, exts)
        && app.run(command, json!({ "path": path })).is_some()
    {
        app.say(tr("Imported"));
    }
}

pub fn format_duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "—".into();
    }
    let s = seconds.round() as u64;
    match (s / 3600, (s % 3600) / 60, s % 60) {
        (0, 0, s) => trf!("{s}s", s = s),
        (0, m, s) => trf!("{m}m {s}s", m = m, s = format!("{s:02}")),
        (h, m, s) => trf!("{h}h {m}m {s}s", h = h, m = format!("{m:02}"), s = format!("{s:02}")),
    }
}

/// "Open Recent" in a browser: the projects it kept, by name.
fn browser_recent_menu(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let mut chosen = None;
    let mut clear = false;
    ui.add_enabled_ui(!app.browser_recent.entries.is_empty(), |ui| {
        ui.menu_button(tr("Open Recent"), |ui| {
            for entry in &app.browser_recent.entries {
                if ui.button(&entry.name).on_hover_text(tr("Kept in this browser")).clicked() {
                    chosen = Some(entry.name.clone());
                    ui.close();
                }
            }
            ui.separator();
            if ui.button(tr("Clear list")).clicked() {
                clear = true;
                ui.close();
            }
        });
    });
    if clear {
        app.browser_recent.entries.clear();
        app.save_browser_recent();
    }
    if let Some(name) = chosen {
        app.request(PendingAction::OpenStored(name));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn durations_follow_the_active_catalog() {
        let t = crate::i18n::parse(r#"{"strings":{"{m}m {s}s":"{m} Min. {s} Sek."}}"#).unwrap();
        crate::i18n::set_active(Some(t));
        assert_eq!(format_duration(125.0), "2 Min. 05 Sek.");
        assert_eq!(format_duration(5.0), "5s", "entries the catalog lacks stay English");
        crate::i18n::set_active(None);
        assert_eq!(format_duration(125.0), "2m 05s");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(5.2), "5s");
        assert_eq!(format_duration(125.0), "2m 05s");
        assert_eq!(format_duration(3725.0), "1h 02m 05s");
        assert_eq!(format_duration(f64::NAN), "—");
        assert_eq!(format_duration(-1.0), "—");
    }
}
