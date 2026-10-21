//! The font manager: every font text can use, each drawn as a sample (`text.preview`), with a button to
//! apply it to the selected text. Fonts come from the built-in stroke font, the downloaded font library
//! and any folder of .ttf/.otf files the person points at.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use laserfoundry_doc::ShapeKind;
use serde_json::{Value, json};

use crate::LaserFoundryApp;
use crate::i18n::{tr, trf};

/// Most font files listed from a chosen folder.
const MAX_FOLDER_FONTS: usize = 400;
/// Height of one row, in points.
const ROW_HEIGHT: f32 = 46.0;
/// Size of the sample area in a row, in points.
const SAMPLE_SIZE: egui::Vec2 = egui::vec2(280.0, 36.0);
/// Samples laid out in one frame at most (large fonts take a moment); the rest wait for the next frame.
const PREVIEWS_PER_FRAME: usize = 2;
/// The sample text shown when the person has typed none.
const DEFAULT_SAMPLE: &str = "Aa Bb Cc 123";
/// Longest sample the box accepts, matching the command's limit.
const MAX_SAMPLE_CHARS: usize = 100;

/// The window's state.
#[derive(Default)]
pub struct FontManager {
    /// The sample text being typed.
    pub sample: String,
    /// The folder whose .ttf/.otf files are listed, and those files as (display name, path).
    pub folder: Option<PathBuf>,
    pub folder_fonts: Vec<(String, String)>,
    /// Drawn samples by font value; cleared when the sample text changes.
    pub(crate) cache: HashMap<String, Preview>,
}

/// A laid-out sample or the reason there is none.
#[derive(Debug, Clone, PartialEq)]
pub enum Preview {
    Ready { paths: Vec<(bool, Vec<[f32; 2]>)>, width: f32, height: f32 },
    Failed(String),
}

/// One row of the list.
struct Entry {
    name: String,
    /// The `font` parameter for the text commands ("" is the built-in font).
    value: String,
    source: &'static str,
}

/// Reads a `text.preview` result. `None` when it is not the shape the command returns.
pub fn parse_preview(v: &Value) -> Option<Preview> {
    let width = v.get("width")?.as_f64()? as f32;
    let height = v.get("height")?.as_f64()? as f32;
    let mut paths = Vec::new();
    for p in v.get("paths")?.as_array()? {
        let points: Vec<[f32; 2]> = p
            .get("points")?
            .as_array()?
            .iter()
            .filter_map(|pt| Some([pt.get(0)?.as_f64()? as f32, pt.get(1)?.as_f64()? as f32]))
            .filter(|pt| pt[0].is_finite() && pt[1].is_finite())
            .collect();
        paths.push((p.get("closed")?.as_bool()?, points));
    }
    Some(Preview::Ready { paths, width, height })
}

/// The .ttf and .otf files directly inside `dir` (not following links), as (file name, full path), sorted
/// and capped.
pub fn scan_font_folder(dir: &Path) -> Vec<(String, String)> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(String, String)> = read
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf")))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().display().to_string()))
        .collect();
    found.sort();
    found.truncate(MAX_FOLDER_FONTS);
    found
}

/// Shows the window when `fonts_open` is set.
pub(crate) fn window(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.fonts_open {
        return;
    }
    let mut open = true;
    let mut apply: Option<String> = None;
    egui::Window::new(tr("Font Manager")).open(&mut open).collapsible(false).default_size([470.0, 420.0]).show(ctx, |ui| {
        let entries = entries(app);
        let current = selected_text_font(app);
        controls(app, ui);
        ui.separator();
        let sample = if app.font_manager.sample.trim().is_empty() { DEFAULT_SAMPLE.to_string() } else { app.font_manager.sample.clone() };
        let mut budget = PREVIEWS_PER_FRAME;
        let mut pending = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_HEIGHT, entries.len(), |ui, range| {
            for entry in entries.get(range).unwrap_or(&[]) {
                if !app.font_manager.cache.contains_key(&entry.value) {
                    if budget == 0 {
                        pending = true;
                    } else {
                        budget -= 1;
                        let preview = lay_out(app, &entry.value, &sample);
                        app.font_manager.cache.insert(entry.value.clone(), preview);
                    }
                }
                ui.horizontal(|ui| {
                    ui.set_height(ROW_HEIGHT - 6.0);
                    ui.vertical(|ui| {
                        ui.set_width(130.0);
                        ui.label(egui::RichText::new(&entry.name).strong());
                        ui.weak(tr(entry.source));
                    });
                    draw_sample(ui, app.font_manager.cache.get(&entry.value));
                    let is_current = current.as_ref() == Some(&entry.value);
                    let can_apply = current.is_some() && !is_current;
                    let label = if is_current { tr("In use") } else { tr("Use") };
                    if ui.add_enabled(can_apply, egui::Button::new(label)).on_disabled_hover_text(tr("Select one text object first")).clicked() {
                        apply = Some(entry.value.clone());
                    }
                });
            }
        });
        if pending {
            ui.ctx().request_repaint();
        }
    });
    if let Some(font) = apply
        && let Some(v) = app.run("text.edit", json!({ "font": font }))
    {
        app.note_unsupported(&v);
        app.text_buffer = None;
    }
    if !open {
        app.fonts_open = false;
        // Fonts may be downloaded or removed while it is closed, so samples are drawn afresh next time.
        app.font_manager.cache.clear();
    }
}

/// The sample box, the folder chooser and the count.
fn controls(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(tr("Sample"));
        let before = app.font_manager.sample.clone();
        ui.add(egui::TextEdit::singleline(&mut app.font_manager.sample).hint_text(DEFAULT_SAMPLE).desired_width(220.0));
        let sample = &mut app.font_manager.sample;
        if sample.chars().count() > MAX_SAMPLE_CHARS {
            *sample = sample.chars().take(MAX_SAMPLE_CHARS).collect();
        }
        if *sample != before {
            app.font_manager.cache.clear();
        }
    });
    if !app.platform_browser_files() {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(tr("Fonts from a folder…"))
                .on_hover_text(tr("Pick any .ttf or .otf file; the font files beside it are listed too"))
                .clicked()
                && let Some(file) = app.pick_open("Choose a font in the folder", &["ttf", "otf"])
                && let Some(dir) = file.parent()
            {
                app.font_manager.folder = Some(dir.to_path_buf());
                app.font_manager.folder_fonts = scan_font_folder(dir);
                app.font_manager.cache.clear();
            }
            if let Some(dir) = app.font_manager.folder.clone() {
                ui.weak(trf!("{count} fonts in {folder}", count = app.font_manager.folder_fonts.len(), folder = dir.display().to_string()));
                if ui.small_button(tr("Clear")).clicked() {
                    app.font_manager.folder = None;
                    app.font_manager.folder_fonts.clear();
                }
            }
        });
    }
}

/// Everything listed: the built-in font, the font library, then the chosen folder.
fn entries(app: &LaserFoundryApp) -> Vec<Entry> {
    let mut out = vec![Entry { name: tr("Built-in stroke font"), value: String::new(), source: "Built in" }];
    out.extend(app.platform_library_fonts().into_iter().map(|(name, value)| Entry { name, value, source: "Font library" }));
    out.extend(app.font_manager.folder_fonts.iter().map(|(name, value)| Entry { name: name.clone(), value: value.clone(), source: "Your folder" }));
    out
}

/// The font of the one selected text object ("" for the built-in font), or `None` when the selection
/// is not exactly one text object.
pub(crate) fn selected_text_font(app: &LaserFoundryApp) -> Option<String> {
    let [id] = app.session.selection.as_slice() else { return None };
    match &app.session.doc.shape(*id)?.kind {
        ShapeKind::Text { style, .. } => Some(style.font.clone().unwrap_or_default()),
        _ => None,
    }
}

fn lay_out(app: &mut LaserFoundryApp, font: &str, sample: &str) -> Preview {
    match app.session.execute("text.preview", &json!({ "font": font, "text": sample })) {
        Ok(v) => parse_preview(&v).unwrap_or_else(|| Preview::Failed(tr("unreadable preview"))),
        Err(e) => Preview::Failed(crate::i18n::tr_message(&e.to_string())),
    }
}

/// Draws a sample as outlines, scaled to fit the box and centred vertically; a failure is shown as text.
fn draw_sample(ui: &mut egui::Ui, preview: Option<&Preview>) {
    let (rect, _) = ui.allocate_exact_size(SAMPLE_SIZE, egui::Sense::hover());
    let colour = ui.visuals().text_color();
    match preview {
        None => {
            ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, "…", egui::FontId::proportional(14.0), colour);
        }
        Some(Preview::Failed(why)) => {
            let galley = ui.painter().layout(why.clone(), egui::FontId::proportional(11.0), ui.visuals().error_fg_color, rect.width());
            ui.painter().galley(rect.left_top(), galley, colour);
        }
        Some(Preview::Ready { paths, width, height }) => {
            let scale = fit_scale(*width, *height, rect.size());
            let origin = rect.left_center() - egui::vec2(0.0, height * scale / 2.0);
            let stroke = egui::Stroke::new(1.0, colour);
            for (closed, points) in paths {
                let pts: Vec<egui::Pos2> = points.iter().map(|p| origin + egui::vec2(p[0] * scale, p[1] * scale)).collect();
                if pts.len() >= 2 {
                    ui.painter().add(if *closed { egui::Shape::closed_line(pts, stroke) } else { egui::Shape::line(pts, stroke) });
                }
            }
        }
    }
}

/// Points per mm that fit a `width` x `height` mm sample into `area` (0 for an empty or invalid one).
pub fn fit_scale(width: f32, height: f32, area: egui::Vec2) -> f32 {
    if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    (area.x / width).min(area.y / height).min(4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_result_is_read_into_scaled_outlines_and_junk_is_refused() {
        let v = json!({"width": 20.0, "height": 10.0, "unsupported": [], "paths": [
            {"closed": true, "points": [[0.0, 0.0], [20.0, 0.0], [20.0, 10.0]]}, {"closed": false, "points": [[1.0, 1.0], [2.0, 2.0]]}]});
        let Some(Preview::Ready { paths, width, height }) = parse_preview(&v) else { panic!("should parse") };
        assert_eq!((paths.len(), width, height), (2, 20.0, 10.0));
        assert_eq!(paths[0], (true, vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0]]));
        for junk in [
            json!(null),
            json!({}),
            json!({"width": 1, "height": 1}),
            json!({"width": 1, "height": 1, "paths": [{"points": []}]}),
            json!({"width": "x", "height": 1, "paths": []}),
        ] {
            assert_eq!(parse_preview(&junk), None, "{junk}");
        }
        // Non-numeric points are skipped rather than drawn.
        let v = json!({"width": 1, "height": 1, "paths": [{"closed": false, "points": [[0, 0], ["a", 1], [2]]}]});
        assert_eq!(parse_preview(&v), Some(Preview::Ready { paths: vec![(false, vec![[0.0, 0.0]])], width: 1.0, height: 1.0 }));
    }

    #[test]
    fn a_sample_is_scaled_to_fit_its_box_and_never_blown_up() {
        let area = egui::vec2(280.0, 36.0);
        assert_eq!(fit_scale(10.0, 10.0, area), 3.6, "height limits a squarish sample");
        assert_eq!(fit_scale(200.0, 10.0, area), 1.4, "width limits a long one");
        assert_eq!(fit_scale(1.0, 1.0, area), 4.0, "a tiny sample is not enlarged without bound");
        for (w, h) in [(0.0, 5.0), (5.0, 0.0), (f32::NAN, 5.0), (5.0, f32::INFINITY), (-1.0, 5.0)] {
            assert_eq!(fit_scale(w, h, area), 0.0, "{w} x {h}");
        }
    }

    #[test]
    fn folder_scan_lists_only_font_files_beside_the_chosen_one_sorted() {
        let dir = std::env::temp_dir().join(format!("lf-fontscan-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub.ttf")).unwrap();
        for name in ["b.ttf", "A.OTF", "c.txt", "d.ttf.bak", "e"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::write(dir.join("sub.ttf").join("inner.ttf"), b"x").unwrap();
        let found: Vec<String> = scan_font_folder(&dir).into_iter().map(|(n, _)| n).collect();
        assert_eq!(found, vec!["A.OTF", "b.ttf"], "a folder named like a font and nested files are not listed");
        assert!(scan_font_folder(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
