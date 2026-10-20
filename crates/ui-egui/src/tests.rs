//! Headless frame tests: real egui input events through the real layout, no window.

use egui::{Event, Modifiers, PointerButton, RawInput, pos2, vec2};
use laserfoundry_geom::Point;
use serde_json::json;

use super::*;

const SCREEN: egui::Vec2 = vec2(1280.0, 800.0);

struct Harness {
    ctx: egui::Context,
    app: LaserFoundryApp,
    time: f64,
    /// Modifier keys held during the frames that follow (announced to egui when they change).
    modifiers: Modifiers,
    sent_modifiers: Modifiers,
}

impl Harness {
    fn new() -> Self {
        let mut h = Harness {
            ctx: egui::Context::default(),
            app: LaserFoundryApp::new(Session::new(), Box::new(NoPlatform)),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, mut events: Vec<Event>) {
        self.time += 0.016;
        if self.modifiers != self.sent_modifiers {
            events.insert(0, Event::ModifiersChanged(self.modifiers));
            self.sent_modifiers = self.modifiers;
        }
        let input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), SCREEN)), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        self.ctx.run_ui(input, |ui| app.ui(ui)).drop_without_applying_deltas();
    }

    /// Screen position of a workspace point, using the view the app settled on.
    fn at(&self, x: f64, y: f64) -> egui::Pos2 {
        self.app.view.map(|v| v.to_screen(Point::new(x, y))).unwrap_or_default()
    }

    fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
        let button = PointerButton::Primary;
        self.frame(vec![Event::PointerMoved(from)]);
        self.frame(vec![Event::PointerButton { pos: from, button, pressed: true, modifiers: Modifiers::NONE }]);
        let mid = from + (to - from) * 0.5;
        self.frame(vec![Event::PointerMoved(mid)]);
        self.frame(vec![Event::PointerMoved(to)]);
        self.frame(vec![Event::PointerButton { pos: to, button, pressed: false, modifiers: Modifiers::NONE }]);
        self.frame(vec![]);
    }

    fn click(&mut self, at: egui::Pos2) {
        self.drag(at, at);
    }

    /// Runs frames until the view has stopped being refitted after a layout change.
    fn settle(&mut self) {
        for _ in 0..8 {
            self.frame(vec![]);
        }
    }
}

/// A platform with a private recovery folder, and canned answers for dialogs.
struct RecoveryPlatform {
    file: std::path::PathBuf,
    open_answer: Option<std::path::PathBuf>,
    recent: Option<std::path::PathBuf>,
}

impl Platform for RecoveryPlatform {
    fn pick_open(&self, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
        self.open_answer.clone()
    }

    fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
        None
    }

    fn recovery_file(&self) -> Option<std::path::PathBuf> {
        Some(self.file.clone())
    }

    fn recent_file(&self) -> Option<std::path::PathBuf> {
        self.recent.clone()
    }

    fn job_position_file(&self) -> Option<std::path::PathBuf> {
        Some(self.file.with_file_name("job-position.json"))
    }

    fn translation_file(&self) -> Option<std::path::PathBuf> {
        Some(self.file.with_file_name("translation.json"))
    }

    fn clear_recovery(&self) {
        let _ = std::fs::remove_file(&self.file);
    }

    fn serial_ports(&self) -> Vec<String> {
        Vec::new()
    }

    fn open_serial(&self, _: &str) -> Result<Box<dyn laserfoundry_device::Transport + Send>, String> {
        Err("none".into())
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lf-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn harness_with(platform: RecoveryPlatform) -> Harness {
    let mut h = Harness {
        ctx: egui::Context::default(),
        app: LaserFoundryApp::new(Session::new(), Box::new(platform)),
        time: 0.0,
        modifiers: Modifiers::NONE,
        sent_modifiers: Modifiers::NONE,
    };
    h.frame(vec![]);
    h.frame(vec![]);
    h
}

mod translation_tests {
    use super::*;
    use crate::i18n;

    #[test]
    fn a_chosen_translation_is_kept_used_at_next_start_and_removed_by_going_back_to_english() {
        let dir = temp_dir("translation");
        let catalog = dir.join("de.json");
        std::fs::write(&catalog, r#"{"name":"Testsprache","strings":{"i18n-ui-test-key":"Übersetzt"}}"#).unwrap();
        let kept = dir.join("translation.json");
        let platform = RecoveryPlatform { file: dir.join("recovery.laserfoundry"), open_answer: Some(catalog.clone()), recent: None };
        let mut h = harness_with(platform);
        h.app.choose_translation();
        assert_eq!(i18n::tr("i18n-ui-test-key"), "Übersetzt");
        assert_eq!(i18n::active_name().as_deref(), Some("Testsprache"));
        assert!(kept.exists(), "a copy is kept for the next start");
        h.frame(vec![]);
        h.frame(vec![]); // the menus and panels draw with a translation active

        let h2 = harness_with(RecoveryPlatform { file: dir.join("recovery.laserfoundry"), open_answer: None, recent: None });
        assert_eq!(i18n::tr("i18n-ui-test-key"), "Übersetzt", "loaded at start");
        drop(h2);

        h.app.use_english();
        assert_eq!(i18n::tr("i18n-ui-test-key"), "i18n-ui-test-key");
        assert!(!kept.exists());
        h.frame(vec![]);

        // A damaged catalog is reported, not used.
        std::fs::write(&catalog, "nope").unwrap();
        h.app.choose_translation();
        assert!(matches!(&h.app.status, Some((_, true))));
        assert_eq!(i18n::active_name(), None);
    }
}

mod engine_message_tests {
    use super::*;

    /// Every error the engine gives for hostile input should be covered by the shipped list of
    /// message templates, so a translator can translate it.
    #[test]
    fn the_shipped_message_list_covers_the_errors_commands_really_give() {
        let strings: String = crate::i18n::ENGINE_MESSAGES.lines().map(|l| format!("{}:\"@@\"", serde_json::json!(l))).collect::<Vec<_>>().join(",");
        let catalog = crate::i18n::parse(&format!("{{\"strings\":{{{strings}}}}}")).unwrap();
        let hostile = [
            serde_json::Value::Null,
            json!({}),
            json!([]),
            json!("string"),
            json!({"x": f64::MAX, "width": 1e308, "height": 1e-308}),
            json!({"ids": [0, 1, 99999], "dx": 1e300}),
            json!({"width": 0, "height": 0, "radius": 0, "sides": 4294967295u64, "degrees": 1e308}),
            json!({"id": -1, "layer": 4294967295u64, "speed": "fast"}),
            json!({"id": 999, "layer": 999, "text": "x", "path": 1, "line": 99999}),
            json!({"path": "/nonexistent/x", "data": "abc"}),
        ];
        let mut unmatched = std::collections::BTreeSet::new();
        for spec in laserfoundry_engine::commands() {
            for params in &hostile {
                let mut session = Session::new();
                session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
                if let Err(e) = session.execute(spec.id, params) {
                    let text = e.to_string();
                    if !catalog.translate_message(&text).contains("@@") {
                        unmatched.insert(text);
                    }
                }
            }
        }
        assert!(unmatched.is_empty(), "errors with no template in engine-messages.txt:\n{}", unmatched.into_iter().collect::<Vec<_>>().join("\n"));
    }

    #[test]
    fn errors_from_the_engine_show_in_the_chosen_language_with_their_values() {
        let dir = temp_dir("engine-messages");
        let catalog = dir.join("xx.json");
        std::fs::write(
            &catalog,
            r#"{"name":"Testsprache","strings":{"layer {} does not exist":"Ebene {1} fehlt (i18n-engine-test)","bad parameters: {0}":"falsch: {0}","nothing is selected":"nichts gewaehlt (i18n-engine-test)"}}"#,
        )
        .unwrap();
        let mut h = harness_with(RecoveryPlatform { file: dir.join("recovery.laserfoundry"), open_answer: Some(catalog), recent: None });
        h.app.choose_translation();
        h.app.run("edit.delete", json!({}));
        assert!(matches!(&h.app.status, Some((m, true)) if m == "nichts gewaehlt (i18n-engine-test)"), "{:?}", h.app.status);
        h.app.run("layer.set", json!({"id": 999, "speed": 100}));
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("Ebene 999 fehlt") || m.contains("falsch")), "{:?}", h.app.status);
        h.app.use_english();
        h.app.run("edit.delete", json!({}));
        assert!(matches!(&h.app.status, Some((m, true)) if m == "nothing is selected"), "{:?}", h.app.status);
    }
}

mod job_position_tests {
    use super::*;
    use crate::job_position::{self, JobPosition};

    fn platform(dir: &std::path::Path) -> RecoveryPlatform {
        RecoveryPlatform { file: dir.join("recovery.laserfoundry"), open_answer: None, recent: None }
    }

    #[test]
    fn a_position_left_by_an_earlier_session_is_offered_and_can_be_forgotten() {
        let dir = temp_dir("jobpos-offer");
        let file = dir.join("job-position.json");
        job_position::save(&file, &JobPosition { hash: 7, line: 12, total_lines: 40 });
        let mut h = harness_with(platform(&dir));
        assert_eq!(h.app.interrupted, Some(JobPosition { hash: 7, line: 12, total_lines: 40 }));
        h.frame(vec![]); // the banner draws, and cannot be used for a different design
        h.app.forget_interrupted();
        assert!(h.app.interrupted.is_none() && !file.exists());
        let h2 = harness_with(platform(&dir));
        assert!(h2.app.interrupted.is_none());
    }

    #[test]
    fn a_clean_run_leaves_no_position_behind_and_starting_a_job_replaces_an_old_one() {
        let dir = temp_dir("jobpos-clean");
        let file = dir.join("job-position.json");
        job_position::save(&file, &JobPosition { hash: 7, line: 3, total_lines: 9 });
        let mut h = harness_with(platform(&dir));
        h.app.connect();
        let start = Instant::now();
        while h.app.snap.as_ref().and_then(|s| s.status.as_ref()).is_none() {
            h.frame(vec![]);
            assert!(start.elapsed() < Duration::from_secs(10), "no status");
            std::thread::sleep(Duration::from_millis(5));
        }
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
        h.app.run("job.gcode", json!({}));
        let program = h.app.session.last_gcode.clone().unwrap();
        h.app.start_job(program);
        assert!(h.app.interrupted.is_none() && !file.exists(), "a new job supersedes the old position");
        while !matches!(h.app.snap.as_ref().map(|s| &s.job), Some(laserfoundry_device::JobState::Finished(_))) {
            h.frame(vec![]);
            assert!(start.elapsed() < Duration::from_secs(20), "job did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        h.frame(vec![]);
        assert!(!file.exists(), "finished cleanly");
    }
}

mod recovery_tests {
    use super::*;

    #[test]
    fn changes_are_autosaved_and_the_copy_is_removed_once_saved() {
        let dir = temp_dir("autosave");
        let file = dir.join("recovery.laserfoundry");
        let mut h = harness_with(RecoveryPlatform { file: file.clone(), open_answer: None, recent: None });
        h.app.autosave_interval = Duration::ZERO;
        h.frame(vec![]);
        assert!(!file.exists(), "nothing to save yet");
        h.app.run("shape.rectangle", json!({"width": 20, "height": 10}));
        h.frame(vec![]);
        assert!(file.exists(), "a changed document is written to the recovery file");
        let copy = laserfoundry_format::load(&file).unwrap();
        assert_eq!(copy.shapes.len(), 1);
        h.app.run("shape.ellipse", json!({"cx": 50, "cy": 50, "rx": 5, "ry": 5}));
        h.frame(vec![]);
        assert_eq!(laserfoundry_format::load(&file).unwrap().shapes.len(), 2, "it follows later edits");
        let saved = dir.join("saved.laserfoundry");
        h.app.run("file.save", json!({"path": saved}));
        h.frame(vec![]);
        assert!(!file.exists(), "saving removes the recovery copy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_idle_window_still_gets_a_repaint_scheduled_so_autosave_can_fire() {
        let dir = temp_dir("repaint");
        let mut h = harness_with(RecoveryPlatform { file: dir.join("recovery.laserfoundry"), open_answer: None, recent: None });
        h.app.run("shape.rectangle", json!({"width": 10, "height": 10}));
        h.app.last_autosave = Instant::now(); // pretend we just autosaved: the next one is due in the future
        h.time += 0.016;
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), SCREEN)), time: Some(h.time), ..Default::default() };
        let app = &mut h.app;
        let output = h.ctx.run_ui(input, |ui| app.ui(ui));
        let delay = output.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay).unwrap_or(Duration::MAX);
        output.drop_without_applying_deltas();
        assert!(delay <= AUTOSAVE_INTERVAL, "a repaint must be requested within the autosave interval, got {delay:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_leftover_copy_is_offered_and_recovered_as_unsaved_work() {
        let dir = temp_dir("recover");
        let file = dir.join("recovery.laserfoundry");
        let mut previous = Session::new();
        previous.execute("shape.rectangle", &json!({"width": 30, "height": 30})).unwrap();
        previous.execute("file.autosave", &json!({"path": file})).unwrap();

        let mut h = harness_with(RecoveryPlatform { file: file.clone(), open_answer: None, recent: None });
        assert!(h.app.pending_recovery.is_some(), "found at startup");
        h.app.autosave_interval = Duration::ZERO;
        h.app.run("shape.line", json!({"x1": 0, "y1": 0, "x2": 5, "y2": 5})); // typing behind the prompt must not clobber the copy
        h.frame(vec![]);
        assert_eq!(laserfoundry_format::load(&file).unwrap().shapes.len(), 1, "the previous session's copy is untouched");

        h.app.answer_recovery(true);
        assert!(h.app.pending_recovery.is_none());
        assert_eq!(h.app.session.doc.shapes.len(), 1);
        assert!(h.app.session.dirty && h.app.session.path.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discarding_the_offer_removes_the_copy() {
        let dir = temp_dir("discard");
        let file = dir.join("recovery.laserfoundry");
        Session::new().execute("file.autosave", &json!({"path": file})).unwrap();
        let mut h = harness_with(RecoveryPlatform { file: file.clone(), open_answer: None, recent: None });
        assert!(h.app.pending_recovery.is_some());
        h.app.answer_recovery(false);
        assert!(h.app.pending_recovery.is_none() && !file.exists());
        assert!(h.app.session.doc.shapes.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_recovery_files_are_reported_not_fatal() {
        let dir = temp_dir("corrupt");
        let file = dir.join("recovery.laserfoundry");
        std::fs::write(&file, b"half a project").unwrap();
        let mut h = harness_with(RecoveryPlatform { file: file.clone(), open_answer: None, recent: None });
        h.app.answer_recovery(true);
        assert!(matches!(&h.app.status, Some((_, true))), "the failure is shown");
        assert!(h.app.session.doc.shapes.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod unsaved_changes_tests {
    use super::*;

    #[test]
    fn new_on_a_clean_document_just_happens_but_a_dirty_one_asks() {
        let dir = temp_dir("confirm");
        let mut h = harness_with(RecoveryPlatform { file: dir.join("r.laserfoundry"), open_answer: None, recent: None });
        h.app.request(PendingAction::New);
        assert!(h.app.pending_action.is_none(), "nothing to lose");
        h.app.run("shape.rectangle", json!({"width": 10, "height": 10}));
        h.app.request(PendingAction::New);
        assert_eq!(h.app.pending_action, Some(PendingAction::New));
        assert_eq!(h.app.session.doc.shapes.len(), 1, "the document is untouched while the question is open");
        h.frame(vec![]); // the dialog renders
        assert!(h.app.has_modal());
        h.app.answer_unsaved(Answer::Cancel);
        assert!(h.app.pending_action.is_none() && h.app.session.doc.shapes.len() == 1, "cancel changes nothing");
        h.app.request(PendingAction::New);
        h.app.answer_unsaved(Answer::Save); // no file dialog answer and no path: nothing gets saved
        assert_eq!(h.app.pending_action, Some(PendingAction::New), "a save that did not happen keeps the question open");
        assert_eq!(h.app.session.doc.shapes.len(), 1);
        h.app.answer_unsaved(Answer::Discard);
        assert!(h.app.session.doc.shapes.is_empty() && h.app.pending_action.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_first_then_continuing_works_when_the_document_has_a_path() {
        let dir = temp_dir("savefirst");
        let target = dir.join("work.laserfoundry");
        let mut h = harness_with(RecoveryPlatform { file: dir.join("r.laserfoundry"), open_answer: None, recent: None });
        h.app.run("shape.rectangle", json!({"width": 10, "height": 10}));
        h.app.run("file.save", json!({"path": target}));
        h.app.run("shape.rectangle", json!({"width": 4, "height": 4}));
        h.app.request(PendingAction::New);
        h.app.answer_unsaved(Answer::Save);
        assert!(h.app.session.doc.shapes.is_empty(), "the new document replaced it");
        assert_eq!(laserfoundry_format::load(&target).unwrap().shapes.len(), 2, "after the changes were saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shortcuts_are_blocked_while_a_question_is_open() {
        let dir = temp_dir("modal");
        let mut h = harness_with(RecoveryPlatform { file: dir.join("r.laserfoundry"), open_answer: None, recent: None });
        h.app.run("shape.rectangle", json!({"width": 10, "height": 10}));
        h.app.request(PendingAction::New);
        h.frame(vec![Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        assert_eq!(h.app.session.doc.shapes.len(), 1, "Delete must not act behind the dialog");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_title_shows_the_name_and_an_unsaved_marker() {
        let mut s = Session::new();
        assert_eq!(window_title(&s), "Untitled — LaserFoundry");
        s.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap();
        assert_eq!(window_title(&s), "*Untitled — LaserFoundry");
        s.path = Some("/tmp/boxes.laserfoundry".into());
        s.dirty = false;
        assert_eq!(window_title(&s), "boxes.laserfoundry — LaserFoundry");
    }

    #[test]
    fn a_close_request_on_unsaved_work_becomes_a_question() {
        let dir = temp_dir("close");
        let mut h = harness_with(RecoveryPlatform { file: dir.join("r.laserfoundry"), open_answer: None, recent: None });
        h.app.run("shape.rectangle", json!({"width": 10, "height": 10}));
        h.time += 0.016;
        let mut input = RawInput { screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), SCREEN)), time: Some(h.time), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        let app = &mut h.app;
        h.ctx.run_ui(input, |ui| app.ui(ui)).drop_without_applying_deltas();
        assert_eq!(h.app.pending_action, Some(PendingAction::Close));
        assert!(!h.app.allow_close);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod frame_tests {
    use super::*;

    #[test]
    fn an_empty_document_renders() {
        let h = Harness::new();
        assert!(h.app.view.is_some());
    }

    #[test]
    fn a_busy_document_renders_with_preview_and_travel() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 30}));
        h.app.run("shape.ellipse", json!({"cx": 100, "cy": 100, "rx": 20, "ry": 10, "layer": 2}));
        h.app.run("layer.set", json!({"id": 0, "mode": "fill_and_line", "interval": 1.0}));
        h.app.show_preview = true;
        h.app.show_travel = true;
        h.frame(vec![]);
        assert!(h.app.preview.error.is_none(), "{:?}", h.app.preview.error);
        assert!(!h.app.preview.segments.is_empty());
    }

    #[test]
    fn a_failing_plan_is_shown_not_hidden() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 1, "y": 1, "width": 5, "height": 5}));
        h.app.session.doc.layers[0].speed = f64::NAN; // bypasses command validation on purpose
        h.app.session.revision += 1;
        h.app.show_preview = true;
        h.frame(vec![]);
        assert!(h.app.preview.error.is_some());
    }
}

mod tool_tests {
    use super::*;

    #[test]
    fn rectangle_tool_creates_a_rectangle_of_the_dragged_size() {
        let mut h = Harness::new();
        h.app.tool = Tool::Rectangle;
        h.drag(h.at(50.0, 50.0), h.at(90.0, 80.0));
        assert_eq!(h.app.session.doc.shapes.len(), 1, "{:?}", h.app.status);
        let b = h.app.session.doc.bounds_of(&h.app.session.selection).unwrap();
        assert!((b.width() - 40.0).abs() < 1.0 && (b.height() - 30.0).abs() < 1.0, "{b:?}");
    }

    #[test]
    fn ellipse_line_and_polygon_tools_create_shapes() {
        let mut h = Harness::new();
        for tool in [Tool::Ellipse, Tool::Line, Tool::Polygon] {
            h.app.tool = tool;
            h.drag(h.at(100.0, 100.0), h.at(140.0, 130.0));
        }
        assert_eq!(h.app.session.doc.shapes.len(), 3);
    }

    #[test]
    fn a_tiny_drag_creates_nothing() {
        let mut h = Harness::new();
        h.app.tool = Tool::Rectangle;
        h.click(h.at(50.0, 50.0));
        assert!(h.app.session.doc.shapes.is_empty());
    }

    #[test]
    fn new_shapes_land_on_the_active_layer() {
        let mut h = Harness::new();
        h.app.active_layer = 4;
        h.app.tool = Tool::Rectangle;
        h.drag(h.at(20.0, 20.0), h.at(60.0, 60.0));
        assert_eq!(h.app.session.doc.shapes[0].layer, 4);
    }
}

mod text_tool_tests {
    use super::*;

    #[test]
    fn clicking_with_the_text_tool_places_text_and_returns_to_select() {
        let mut h = Harness::new();
        h.app.tool = Tool::Text;
        h.app.active_layer = 3;
        h.click(h.at(60.0, 70.0));
        assert_eq!(h.app.session.doc.shapes.len(), 1, "{:?}", h.app.status);
        let shape = &h.app.session.doc.shapes[0];
        assert_eq!(shape.layer, 3);
        assert!(matches!(&shape.kind, laserfoundry_doc::ShapeKind::Text { content, .. } if content == "Text"));
        let b = shape.world_bounds().unwrap();
        assert!((b.x0 - 60.0).abs() < 1.5 && (b.y0 - 70.0).abs() < 1.5, "{b:?}");
        assert_eq!(h.app.tool, Tool::Select);
        assert_eq!(h.app.session.selection, vec![shape.id]);
    }

    #[test]
    fn the_inspector_follows_the_selected_text_and_edits_apply() {
        let mut h = Harness::new();
        h.app.run("text.create", json!({"text": "ONE", "x": 10, "y": 10}));
        h.frame(vec![]);
        assert!(matches!(&h.app.text_buffer, Some((_, t)) if t == "ONE"));
        h.app.run("text.edit", json!({"text": "TWO"}));
        h.app.text_buffer = None; // what the inspector does after committing
        h.frame(vec![]);
        assert!(matches!(&h.app.text_buffer, Some((_, t)) if t == "TWO"));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        assert!(h.app.text_buffer.is_none());
    }

    #[test]
    fn text_is_selected_by_clicking_anywhere_in_its_block_but_not_beside_it() {
        let mut h = Harness::new();
        h.app.run("text.create", json!({"text": "TEXT", "x": 100, "y": 100, "height": 20}));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        // Between the stem of the T and the E: no stroke is near, but it is inside the block.
        h.click(h.at(104.0, 112.0));
        assert_eq!(h.app.session.selection.len(), 1, "inside the block");
        h.click(h.at(104.0, 150.0));
        assert!(h.app.session.selection.is_empty(), "well below the block");
    }

    #[test]
    fn unsupported_characters_raise_a_visible_warning() {
        let mut h = Harness::new();
        if let Some(v) = h.app.run("text.create", json!({"text": "caf\u{e9}"})) {
            h.app.note_unsupported(&v);
        }
        assert!(h.app.text_warning.as_deref().is_some_and(|w| w.contains('\u{e9}')));
        if let Some(v) = h.app.run("text.edit", json!({"text": "cafe"})) {
            h.app.note_unsupported(&v);
        }
        assert!(h.app.text_warning.is_none());
    }
}

mod image_tests {
    use std::sync::Arc;

    use laserfoundry_doc::{ImageAdjust, RasterImage, Shape, ShapeKind};
    use laserfoundry_geom::Affine;

    use super::*;

    fn add_gradient(h: &mut Harness, transform: Affine) -> u64 {
        let gray: Vec<u8> = (0..16 * 16).map(|i| ((i % 16) * 16) as u8).collect();
        let image = Arc::new(RasterImage::new(16, 16, gray).unwrap());
        let id = h.app.session.doc.add_shape(Shape {
            id: 0,
            name: "pic".into(),
            layer: 0,
            transform,
            kind: ShapeKind::Image { image, width_mm: 40.0, height_mm: 40.0, adjust: ImageAdjust::default() },
            cut: None,
            group: None,
            order: 0,
        });
        h.app.session.revision += 1;
        id
    }

    #[test]
    fn images_get_a_texture_that_follows_their_adjustment_and_is_dropped_with_the_shape() {
        let mut h = Harness::new();
        let id = add_gradient(&mut h, Affine::translate((20.0, 20.0)));
        h.frame(vec![]);
        assert!(h.app.thumbs.map_len() == 1);
        let before = h.app.thumbs.cached_key(id);
        h.app.run("image.adjust", json!({"id": id, "negative": true}));
        h.frame(vec![]);
        assert_ne!(h.app.thumbs.cached_key(id), before, "a changed adjustment rebuilds the thumbnail");
        let key = h.app.thumbs.cached_key(id);
        h.app.run("edit.move", json!({"ids": [id], "dx": 10, "dy": 0}));
        h.frame(vec![]);
        assert_eq!(h.app.thumbs.cached_key(id), key, "moving does not rebuild it");
        h.app.run("edit.delete", json!({"ids": [id]}));
        h.frame(vec![]);
        assert_eq!(h.app.thumbs.map_len(), 0);
    }

    #[test]
    fn the_crop_and_sharpen_section_renders_for_a_selected_image_and_the_thumbnail_follows_a_crop() {
        let mut h = Harness::new();
        let id = add_gradient(&mut h, Affine::translate((20.0, 20.0)));
        h.app.run("edit.select", json!({"ids": [id]}));
        h.app.crop_pct = [25.0, 0.0, 25.0, 50.0];
        h.frame(vec![]);
        h.frame(vec![]);
        let key = h.app.thumbs.cached_key(id);
        h.app.run("image.crop", json!({"id": id, "left": 0.25, "right": 0.25, "bottom": 0.5}));
        h.frame(vec![]);
        assert_ne!(h.app.thumbs.cached_key(id), key, "a cropped picture gets a new thumbnail");
        let b = h.app.session.doc.shape(id).unwrap().world_bounds().unwrap();
        assert!((b.x1 - b.x0 - 20.0).abs() < 1e-6 && (b.y1 - b.y0 - 20.0).abs() < 1e-6, "{b:?}");
    }

    #[test]
    fn dragging_a_picture_edge_in_crop_mode_sets_the_percentage_and_applying_crops() {
        let mut h = Harness::new();
        h.app.snapping = crate::snap::SnapSettings { grid: None, objects: false, ..Default::default() };
        let id = add_gradient(&mut h, Affine::translate((20.0, 20.0))); // 40 x 40 mm at (20, 20)
        h.app.run("edit.select", json!({"ids": [id]}));
        h.app.crop_mode = true;
        h.frame(vec![]);
        h.frame(vec![]);
        // Grab the left edge (x = 20) and pull it to x = 30: a quarter of the width.
        h.drag(h.at(20.0, 40.0), h.at(30.0, 40.0));
        assert!((h.app.crop_pct[0] - 25.0).abs() < 1.5, "{:?}", h.app.crop_pct);
        assert!(h.app.crop_pct[1..].iter().all(|p| *p == 0.0));
        assert_eq!(h.app.crop_drag, None, "released");
        // Then the bottom edge (y = 60) up to y = 50.
        h.drag(h.at(40.0, 60.0), h.at(40.0, 50.0));
        assert!((h.app.crop_pct[3] - 25.0).abs() < 1.5, "{:?}", h.app.crop_pct);
        assert_eq!(h.app.session.selection, vec![id], "the drag did not move or deselect the picture");
        let before = h.app.session.doc.shape(id).unwrap().world_bounds().unwrap();
        assert_eq!((before.x0, before.y1), (20.0, 60.0));

        let [l, t, r, b] = h.app.crop_pct.map(|p| p / 100.0);
        h.app.run("image.crop", json!({"id": id, "left": l, "top": t, "right": r, "bottom": b}));
        let after = h.app.session.doc.shape(id).unwrap().world_bounds().unwrap();
        assert!((after.x0 - 30.0).abs() < 1.0 && (after.y1 - 50.0).abs() < 1.0, "{after:?}");
        h.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        assert!(!h.app.crop_mode, "Escape leaves crop mode");
    }

    #[test]
    fn crop_percentages_do_not_follow_the_selection_to_another_picture() {
        let mut h = Harness::new();
        let a = add_gradient(&mut h, Affine::translate((20.0, 20.0)));
        let b = add_gradient(&mut h, Affine::translate((120.0, 20.0)));
        h.app.run("edit.select", json!({"ids": [a]}));
        h.frame(vec![]);
        h.app.crop_pct = [10.0, 20.0, 30.0, 40.0];
        h.frame(vec![]);
        assert_eq!(h.app.crop_pct, [10.0, 20.0, 30.0, 40.0], "kept while the same picture stays selected");
        h.app.run("edit.select", json!({"ids": [b]}));
        h.frame(vec![]);
        assert_eq!(h.app.crop_pct, [0.0; 4]);
    }

    #[test]
    fn crop_mode_ends_when_the_selection_is_not_a_single_picture_and_edges_use_the_pictures_own_axes() {
        use crate::crop_ui::{edge_at, pct_for};
        let mut h = Harness::new();
        h.app.crop_mode = true;
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(!h.app.crop_mode, "nothing selected, nothing to crop");
        // A picture turned a quarter: its local left edge is now at the top of the screen.
        let t = Affine::translate((50.0, 50.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
        let view = h.app.view.unwrap();
        let top_of_screen = view.to_screen(t * Point::new(0.0, 20.0)); // middle of the local left edge
        assert_eq!(edge_at(&view, t, 40.0, 40.0, top_of_screen), Some(0));
        assert!((pct_for(0, t, 40.0, 40.0, t * Point::new(10.0, 20.0)) - 25.0).abs() < 1e-9);
        assert_eq!(pct_for(0, t, 40.0, 40.0, Point::new(f64::NAN, 0.0)), 0.0);
        assert_eq!(pct_for(2, t, 40.0, 40.0, t * Point::new(-100.0, 0.0)), 95.0, "capped");
    }

    #[test]
    fn rotated_and_flipped_images_render_and_are_selectable() {
        let mut h = Harness::new();
        let id = add_gradient(&mut h, Affine::translate((80.0, 80.0)) * Affine::rotate(0.7) * Affine::scale_non_uniform(-1.0, 1.0));
        h.frame(vec![]);
        h.frame(vec![]);
        let c = h.app.session.doc.bounds_of(&[id]).unwrap().center();
        h.click(h.at(c.x, c.y));
        assert_eq!(h.app.session.selection, vec![id]);
        h.frame(vec![]); // the Image inspector is drawn for it
    }
}

mod node_tool_tests {
    use super::*;

    fn nodes_of(h: &Harness, id: u64) -> laserfoundry_geom::nodes::NodePath {
        match &h.app.session.doc.shape(id).unwrap().kind {
            laserfoundry_doc::ShapeKind::Path { path } => laserfoundry_geom::nodes::NodePath::from_bez(path).unwrap(),
            other => panic!("{other:?}"),
        }
    }

    fn with_rect() -> (Harness, u64) {
        let mut h = Harness::new();
        h.app.snapping = snap::SnapSettings { grid: None, objects: false, ..Default::default() };
        let id = h.app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap();
        h.app.tool = Tool::Nodes;
        h.frame(vec![]);
        (h, id)
    }

    fn key(key: egui::Key) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn dragging_a_corner_node_reshapes_the_path_in_one_undo_step() {
        let (mut h, id) = with_rect();
        h.drag(h.at(100.0, 100.0), h.at(90.0, 85.0));
        let np = nodes_of(&h, id);
        assert!(np.subpaths[0].nodes[0].p.distance(Point::new(90.0, 85.0)) < 0.2, "{:?}", np.subpaths[0].nodes[0].p);
        assert_eq!(h.app.node_sel.map(|s| s.index), Some(0));
        h.app.run("edit.undo", serde_json::Value::Null);
        assert!(nodes_of(&h, id).subpaths[0].nodes[0].p.distance(Point::new(100.0, 100.0)) < 1e-9);
    }

    #[test]
    fn handles_of_the_selected_node_can_be_dragged_to_bend_a_curve() {
        let (mut h, id) = with_rect();
        h.app.run("node.set_segment", json!({"id": id, "segment": 0, "curve": true}));
        h.click(h.at(100.0, 100.0)); // select node 0
        h.frame(vec![]);
        let handle_tip = nodes_of(&h, id).subpaths[0].nodes[0].p + nodes_of(&h, id).subpaths[0].nodes[0].out_h.unwrap();
        h.drag(h.at(handle_tip.x, handle_tip.y), h.at(125.0, 80.0));
        let np = nodes_of(&h, id);
        let tip = np.subpaths[0].nodes[0].p + np.subpaths[0].nodes[0].out_h.unwrap();
        assert!(tip.distance(Point::new(125.0, 80.0)) < 0.2, "{tip:?}");
        assert!(h.app.session.doc.shape(id).unwrap().world_bounds().unwrap().y0 < 100.0, "the edge now bulges");
    }

    #[test]
    fn double_clicking_a_segment_adds_a_node_there_and_selects_it() {
        let (mut h, id) = with_rect();
        h.click(h.at(130.0, 100.0));
        h.click(h.at(130.0, 100.0));
        let np = nodes_of(&h, id);
        assert_eq!(np.subpaths[0].nodes.len(), 5, "{:?}", h.app.status);
        assert!(np.subpaths[0].nodes[1].p.distance(Point::new(130.0, 100.0)) < 1.0);
        assert_eq!(h.app.node_sel.map(|s| s.index), Some(1));
    }

    #[test]
    fn delete_removes_the_selected_node_but_the_whole_shape_when_no_node_is_selected() {
        let (mut h, id) = with_rect();
        h.click(h.at(160.0, 100.0));
        assert_eq!(h.app.node_sel.map(|s| s.index), Some(1));
        h.frame(vec![key(egui::Key::Delete)]);
        assert_eq!(nodes_of(&h, id).subpaths[0].nodes.len(), 3, "a node, not the shape");
        h.app.node_sel = None;
        h.frame(vec![key(egui::Key::Delete)]);
        assert!(h.app.session.doc.shapes.is_empty(), "no node selected: the shape goes");
    }

    #[test]
    fn clicking_empty_space_deselects_the_node_and_clicking_another_shape_edits_that_one() {
        let (mut h, _) = with_rect();
        let other = h.app.run("shape.ellipse", json!({"cx": 250, "cy": 250, "rx": 20, "ry": 20})).unwrap()["id"].as_u64().unwrap();
        h.app.run("edit.select", json!({"ids": [1]}));
        h.frame(vec![]);
        h.click(h.at(100.0, 100.0));
        assert!(h.app.node_sel.is_some());
        h.click(h.at(300.0, 350.0));
        assert!(h.app.node_sel.is_none());
        h.click(h.at(250.0, 230.0));
        assert_eq!(h.app.session.selection, vec![other], "the ellipse is now the edited path");
        h.frame(vec![]);
    }

    #[test]
    fn node_drags_snap_to_the_grid() {
        let (mut h, id) = with_rect();
        h.app.snapping = snap::SnapSettings { grid: Some(5.0), objects: false, ..Default::default() };
        h.drag(h.at(100.0, 100.0), h.at(87.0, 83.0));
        assert!(nodes_of(&h, id).subpaths[0].nodes[0].p.distance(Point::new(85.0, 85.0)) < 1e-9);
    }

    #[test]
    fn text_shows_a_convert_button_instead_of_nodes_and_conversion_makes_it_editable() {
        let mut h = Harness::new();
        h.app.run("text.create", json!({"text": "HI", "x": 50, "y": 50, "height": 20}));
        h.app.tool = Tool::Nodes;
        h.frame(vec![]);
        assert!(crate::nodes_ui::world_nodes(&h.app).is_none());
        h.app.run("edit.convert_to_path", json!({}));
        h.frame(vec![]);
        assert!(crate::nodes_ui::world_nodes(&h.app).is_some());
    }
}

mod pen_tool_tests {
    use super::*;

    fn pen() -> Harness {
        let mut h = Harness::new();
        h.app.snapping = snap::SnapSettings { grid: None, objects: false, ..Default::default() };
        h.app.tool = Tool::Pen;
        h.frame(vec![]);
        h
    }

    fn key(key: egui::Key) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn clicking_points_and_then_the_first_one_closes_a_polygon_on_the_active_layer() {
        let mut h = pen();
        h.app.active_layer = 5;
        for (x, y) in [(100.0, 100.0), (180.0, 100.0), (140.0, 170.0)] {
            h.click(h.at(x, y));
        }
        assert!(h.app.session.doc.shapes.is_empty(), "still drawing");
        assert_eq!(h.app.pen.as_ref().map(|p| p.nodes.len()), Some(3));
        h.frame(vec![]); // the in-progress path is drawn
        h.click(h.at(100.0, 100.0));
        assert!(h.app.pen.is_none());
        let shape = &h.app.session.doc.shapes[0];
        assert_eq!(shape.layer, 5);
        assert!(shape.world_polylines()[0].closed && shape.world_polylines()[0].points.len() == 3);
    }

    #[test]
    fn dragging_out_of_a_new_node_makes_a_smooth_node_with_mirrored_handles() {
        let mut h = pen();
        h.click(h.at(100.0, 100.0));
        h.drag(h.at(150.0, 100.0), h.at(180.0, 130.0));
        h.click(h.at(220.0, 100.0));
        h.frame(vec![key(egui::Key::Enter)]);
        let shape = &h.app.session.doc.shapes[0];
        let laserfoundry_doc::ShapeKind::Path { path } = &shape.kind else { panic!() };
        let np = laserfoundry_geom::nodes::NodePath::from_bez(path).unwrap();
        let n = &np.subpaths[0].nodes[1];
        assert!(n.p.distance(Point::new(150.0, 100.0)) < 0.5);
        let (i, o) = (n.in_h.unwrap(), n.out_h.unwrap());
        assert!((o.x - 30.0).abs() < 0.5 && (o.y - 30.0).abs() < 0.5, "{o:?}");
        assert!((i + o).hypot() < 1e-6, "mirrored: {i:?} {o:?}");
        assert!(n.smooth && !np.subpaths[0].closed);
    }

    #[test]
    fn enter_finishes_double_click_finishes_and_escape_cancels() {
        let mut h = pen();
        h.click(h.at(100.0, 100.0));
        h.click(h.at(160.0, 120.0));
        h.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(h.app.session.doc.shapes.len(), 1);
        h.click(h.at(200.0, 200.0));
        h.click(h.at(260.0, 220.0));
        h.time += 1.0;
        h.click(h.at(300.0, 260.0));
        h.click(h.at(300.0, 260.0)); // second click of a double click
        assert_eq!(h.app.session.doc.shapes.len(), 2, "double click finished the open path");
        assert_eq!(h.app.session.doc.shapes[1].world_polylines()[0].points.len(), 3, "without a duplicate end node");
        h.click(h.at(100.0, 300.0));
        h.click(h.at(150.0, 300.0));
        h.frame(vec![key(egui::Key::Escape)]);
        assert!(h.app.pen.is_none());
        assert_eq!(h.app.session.doc.shapes.len(), 2, "cancelled");
        assert_eq!(h.app.tool, Tool::Pen, "Escape cancelled the path, it did not leave the tool");
        h.frame(vec![key(egui::Key::Escape)]);
        assert_eq!(h.app.tool, Tool::Select, "a second Escape leaves the tool");
    }

    #[test]
    fn a_single_click_never_makes_a_one_node_path() {
        let mut h = pen();
        h.click(h.at(100.0, 100.0));
        h.frame(vec![key(egui::Key::Enter)]);
        assert!(h.app.session.doc.shapes.is_empty());
        assert!(h.app.pen.is_none());
    }

    #[test]
    fn pen_paths_snap_and_pen_clicks_on_rulers_make_guides_not_nodes() {
        let mut h = pen();
        h.app.snapping = snap::SnapSettings { grid: Some(10.0), objects: false, ..Default::default() };
        h.click(h.at(103.0, 98.0));
        h.click(h.at(147.0, 121.0));
        h.frame(vec![key(egui::Key::Enter)]);
        let pts = &h.app.session.doc.shapes[0].world_polylines()[0].points;
        assert!(pts[0].distance(Point::new(100.0, 100.0)) < 1e-9 && pts[1].distance(Point::new(150.0, 120.0)) < 1e-9, "{pts:?}");
        let c = h.app.canvas_rect.unwrap();
        h.drag(c.min + vec2(5.0, 200.0), c.min + vec2(300.0, 200.0));
        assert_eq!(h.app.guides.len(), 1, "the ruler still produces guides with the pen selected");
        assert!(h.app.pen.is_none());
    }

    #[test]
    fn switching_tools_with_a_path_in_progress_does_not_lose_it() {
        let mut h = pen();
        h.click(h.at(100.0, 100.0));
        h.click(h.at(160.0, 120.0));
        crate::nodes_ui::finish(&mut h.app, false);
        h.app.tool = Tool::Select;
        assert_eq!(h.app.session.doc.shapes.len(), 1);
    }
}

mod snapping_tests {
    use super::*;

    fn last_bounds(h: &Harness) -> laserfoundry_geom::Rect {
        h.app.session.doc.shapes.last().unwrap().world_bounds().unwrap()
    }

    #[test]
    fn grid_snapping_puts_a_created_rectangle_exactly_on_the_grid() {
        let mut h = Harness::new();
        h.app.snapping = snap::SnapSettings { grid: Some(5.0), objects: false, ..Default::default() };
        h.app.tool = Tool::Rectangle;
        h.drag(h.at(52.0, 51.0), h.at(88.0, 96.5));
        let b = last_bounds(&h);
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (50.0, 50.0, 90.0, 95.0), "{b:?}");
    }

    #[test]
    fn without_snapping_the_drag_is_taken_literally() {
        let mut h = Harness::new();
        h.app.snapping = snap::SnapSettings { grid: None, objects: false, ..Default::default() };
        h.app.tool = Tool::Rectangle;
        h.drag(h.at(52.0, 51.0), h.at(88.0, 96.5));
        let b = last_bounds(&h);
        assert!((b.x0 - 52.0).abs() < 0.05 && (b.y1 - 96.5).abs() < 0.05, "{b:?}");
    }

    #[test]
    fn a_new_shape_snaps_to_the_edges_of_an_existing_one() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 40, "height": 40}));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        h.app.tool = Tool::Rectangle;
        // Start 1.5 mm left of the existing rectangle's left edge and 2 mm above its centre line.
        h.drag(h.at(98.5, 118.0), h.at(60.0, 60.0));
        let b = last_bounds(&h);
        assert_eq!(b.x1, 100.0, "the new rectangle's right edge locks to x = 100: {b:?}");
        assert_eq!(b.y1, 120.0, "and its bottom to the centre line y = 120");
    }

    /// Presses on `from`, moves to `to` and leaves the button down, so the drag's overlays can be inspected.
    fn hold_drag(h: &mut Harness, from: egui::Pos2, to: egui::Pos2) {
        h.frame(vec![Event::PointerMoved(from)]);
        h.frame(vec![Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        h.frame(vec![Event::PointerMoved(from + (to - from) * 0.5)]);
        h.frame(vec![Event::PointerMoved(to)]);
        h.frame(vec![]);
    }

    #[test]
    fn a_moving_shape_reports_what_it_is_aligned_with() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 100, "y": 20, "width": 30, "height": 30}));
        h.app.run("shape.rectangle", json!({"x": 20, "y": 120, "width": 30, "height": 30}));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        let (from, to) = (h.at(35.0, 135.0), h.at(35.0 + 78.7, 135.0));
        hold_drag(&mut h, from, to);
        let x = h.app.snap_lines.0.expect("locked onto the other rectangle's left side");
        // A rectangle is a path, so its corner (the more specific target) is what the line comes from.
        assert_eq!((x.at, x.kind), (100.0, snap::TargetKind::Endpoint));
        assert!(matches!(x.span, Some((y, _)) if y == 20.0 || y == 50.0), "the line can be drawn back to that corner: {x:?}");
    }

    #[test]
    fn a_shape_dragged_to_the_middle_of_the_bed_reports_the_bed_centre() {
        let mut h = Harness::new();
        let (bw, bh) = (h.app.session.doc.machine.bed_width, h.app.session.doc.machine.bed_height);
        h.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 20, "height": 20}));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        // Its centre is at (30, 30); drag it to 0.6 mm short of the bed's centre on both axes.
        let (from, to) = (h.at(30.0, 30.0), h.at(bw / 2.0 - 0.6, bh / 2.0 - 0.6));
        hold_drag(&mut h, from, to);
        assert_eq!(h.app.snap_lines.0.map(|t| (t.at, t.kind)), Some((bw / 2.0, snap::TargetKind::BedCenter)));
        assert_eq!(h.app.snap_lines.1.map(|t| (t.at, t.kind)), Some((bh / 2.0, snap::TargetKind::BedCenter)));
    }

    #[test]
    fn a_shape_moved_between_two_others_snaps_to_equal_gaps_and_shows_them() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 20, "height": 20}));
        h.app.run("shape.rectangle", json!({"x": 120, "y": 20, "width": 20, "height": 20}));
        h.app.run("shape.rectangle", json!({"x": 200, "y": 24, "width": 20, "height": 12}));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        // Centred between the others it would span 70..90 (gaps of 30); stop 2 mm short of that.
        let (from, to) = (h.at(210.0, 30.0), h.at(78.0, 30.0));
        hold_drag(&mut h, from, to);
        assert_eq!(h.app.snap_spacing.len(), 2, "{:?}", h.app.snap_spacing);
        assert!(h.app.snap_spacing.iter().all(|g| (g.size - 30.0).abs() < 1e-6), "{:?}", h.app.snap_spacing);
    }

    fn triangle(h: &mut Harness) {
        h.app.run("path.create", json!({"nodes": [{"x": 100.0, "y": 100.0}, {"x": 180.0, "y": 100.0}, {"x": 120.0, "y": 140.0}], "closed": true}));
        h.app.run("edit.select", json!({"ids": []}));
        h.app.tool = Tool::Rectangle;
        h.frame(vec![]);
    }

    #[test]
    fn resting_on_a_vertex_with_a_drawing_tool_acquires_it_for_tracking() {
        let mut h = Harness::new();
        triangle(&mut h);
        let on = h.at(120.0, 140.0);
        h.frame(vec![Event::PointerMoved(on)]);
        assert!(h.app.tracked.is_empty() && h.app.track_candidate.is_some(), "not yet: it has only just arrived");
        for _ in 0..45 {
            h.frame(vec![]);
        }
        assert_eq!(h.app.tracked, vec![Point::new(120.0, 140.0)]);
        // Resting on it again does not toggle it off.
        for _ in 0..45 {
            h.frame(vec![]);
        }
        assert_eq!(h.app.tracked.len(), 1);
    }

    #[test]
    fn passing_over_a_vertex_or_resting_on_a_plain_edge_acquires_nothing() {
        let mut h = Harness::new();
        triangle(&mut h);
        let (a, b) = (h.at(120.0, 140.0), h.at(300.0, 300.0));
        h.frame(vec![Event::PointerMoved(a)]);
        for _ in 0..10 {
            h.frame(vec![]);
        }
        h.frame(vec![Event::PointerMoved(b)]);
        for _ in 0..60 {
            h.frame(vec![]);
        }
        assert!(h.app.tracked.is_empty() && h.app.track_candidate.is_none());
    }

    #[test]
    fn a_shape_started_on_a_tracked_diagonal_snaps_onto_it_and_finishing_forgets_the_point() {
        let mut h = Harness::new();
        triangle(&mut h);
        h.app.tracked.push(Point::new(100.0, 100.0));
        h.app.snapping = snap::SnapSettings { grid: None, objects: true, ..Default::default() };
        h.frame(vec![]);
        // 1.3 mm off the 45-degree line through (100, 100); no object line is near x = 250 or y = 251.3.
        let (from, to) = (h.at(250.0, 251.3), h.at(300.0, 330.0));
        h.drag(from, to);
        let b = last_bounds(&h);
        assert!((b.x0 - 100.0 - (b.y0 - 100.0)).abs() < 1e-6, "exactly 45 degrees from the tracked point: {b:?}");
        assert!(h.app.tracked.is_empty(), "tracking belongs to the shape that was being placed");
    }

    fn press_t(h: &mut Harness) {
        let key = |pressed| Event::Key { key: egui::Key::T, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        h.frame(vec![key(true)]);
        h.frame(vec![key(false)]);
    }

    #[test]
    fn the_nodes_tool_forgets_tracked_points_but_select_keeps_them() {
        let mut h = Harness::new();
        triangle(&mut h);
        h.app.tracked.push(Point::new(100.0, 100.0));
        h.app.tool = Tool::Select;
        h.frame(vec![]);
        assert_eq!(h.app.tracked.len(), 1, "Select can use them to move things");
        h.app.tool = Tool::Nodes;
        h.frame(vec![]);
        assert!(h.app.tracked.is_empty());
    }

    #[test]
    fn pressing_t_over_a_vertex_acquires_it_in_any_tool_and_again_removes_it() {
        let mut h = Harness::new();
        triangle(&mut h);
        h.app.tool = Tool::Select;
        let on = h.at(120.0, 140.0);
        h.frame(vec![Event::PointerMoved(on)]);
        press_t(&mut h);
        assert_eq!(h.app.tracked, vec![Point::new(120.0, 140.0)]);
        press_t(&mut h);
        assert!(h.app.tracked.is_empty(), "T toggles");
        // Over empty canvas there is nothing to acquire.
        let off = h.at(300.0, 300.0);
        h.frame(vec![Event::PointerMoved(off)]);
        press_t(&mut h);
        assert!(h.app.tracked.is_empty());
    }

    #[test]
    fn a_moving_shape_is_pulled_onto_a_tracked_angle_and_the_point_is_forgotten_after() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 210, "y": 210, "width": 20, "height": 20}));
        h.app.run("edit.select", json!({"ids": []}));
        h.app.tool = Tool::Select;
        h.app.snapping = snap::SnapSettings { grid: None, objects: true, ..Default::default() };
        h.app.tracked.push(Point::new(100.0, 100.0));
        h.frame(vec![]);
        // Drag it so its top-left corner ends 1.3 mm below the 45-degree line through (100, 100) at (150, 150).
        let (from, to) = (h.at(220.0, 220.0), h.at(160.0, 161.3));
        hold_drag(&mut h, from, to);
        assert_eq!(h.app.snap_lines.2.map(|d| d.degrees), Some(45.0), "{:?}", h.app.snap_lines);
        let (from, to) = (h.at(160.0, 161.3), h.at(160.0, 161.3));
        let _ = (from, to);
        h.frame(vec![Event::PointerButton { pos: h.at(160.0, 161.3), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        h.frame(vec![]);
        let b = last_bounds(&h);
        assert!(((b.x0 - 100.0) - (b.y0 - 100.0)).abs() < 1e-6, "its corner is exactly on the line: {b:?}");
        assert!(h.app.tracked.is_empty());
    }

    #[test]
    fn a_moving_shape_follows_a_tracked_row() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 210, "y": 210, "width": 20, "height": 20}));
        h.app.run("edit.select", json!({"ids": []}));
        h.app.tool = Tool::Select;
        h.app.snapping = snap::SnapSettings { grid: None, objects: true, ..Default::default() };
        h.app.tracked.push(Point::new(100.0, 77.0));
        h.frame(vec![]);
        let (from, to) = (h.at(220.0, 220.0), h.at(300.0, 77.0 + 10.0 + 1.2));
        hold_drag(&mut h, from, to);
        assert_eq!(h.app.snap_lines.1.map(|t| (t.kind, t.at)), Some((snap::TargetKind::Tracked, 77.0)), "{:?}", h.app.snap_lines);
    }

    #[test]
    fn shift_makes_squares_and_circles() {
        let mut h = Harness::new();
        h.app.snapping = snap::SnapSettings { grid: None, objects: false, ..Default::default() };
        h.app.tool = Tool::Rectangle;
        h.modifiers = Modifiers::SHIFT;
        h.drag(h.at(50.0, 50.0), h.at(110.0, 70.0));
        let b = last_bounds(&h);
        assert!((b.width() - b.height()).abs() < 1e-6 && (b.width() - 60.0).abs() < 0.1, "{b:?}");
        h.app.tool = Tool::Line;
        h.drag(h.at(200.0, 200.0), h.at(260.0, 204.0));
        let line = last_bounds(&h);
        assert!(line.height() < 1e-6, "a nearly horizontal line becomes horizontal: {line:?}");
    }

    #[test]
    fn moving_a_shape_locks_it_to_another_shapes_edge_and_shows_the_alignment() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 100, "y": 20, "width": 30, "height": 30}));
        h.app.run("shape.rectangle", json!({"x": 20, "y": 120, "width": 30, "height": 30}));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        // Drag the second shape right so its left edge ends 1.3 mm short of x = 100.
        h.drag(h.at(35.0, 135.0), h.at(35.0 + 78.7, 135.0));
        let b = h.app.session.doc.shapes[1].world_bounds().unwrap();
        assert!((b.x0 - 100.0).abs() < 1e-6, "left edges aligned: {b:?}");
    }

    #[test]
    fn shift_locks_a_move_to_one_axis() {
        let mut h = Harness::new();
        h.app.snapping = snap::SnapSettings { grid: None, objects: false, ..Default::default() };
        h.app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 30, "height": 30}));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        h.modifiers = Modifiers::SHIFT;
        h.drag(h.at(115.0, 115.0), h.at(145.0, 123.0));
        let b = h.app.session.doc.shapes[0].world_bounds().unwrap();
        assert!((b.x0 - 130.0).abs() < 0.1 && (b.y0 - 100.0).abs() < 1e-6, "{b:?}");
    }
}

mod ruler_and_guide_tests {
    use super::*;

    fn ruler_left(h: &Harness) -> egui::Pos2 {
        let c = h.app.canvas_rect.expect("canvas drawn");
        c.min + vec2(canvas_ruler() / 2.0, 200.0)
    }

    fn ruler_top(h: &Harness) -> egui::Pos2 {
        let c = h.app.canvas_rect.expect("canvas drawn");
        c.min + vec2(300.0, canvas_ruler() / 2.0)
    }

    fn canvas_ruler() -> f32 {
        crate::canvas::RULER_PX
    }

    #[test]
    fn dragging_out_of_the_rulers_makes_guides_and_dragging_back_removes_them() {
        let mut h = Harness::new();
        let target = h.at(150.0, 0.0);
        let to = egui::pos2(target.x, h.app.canvas_rect.unwrap().min.y + 300.0);
        h.drag(ruler_left(&h), to);
        assert_eq!(h.app.guides.len(), 1, "{:?}", h.app.guides);
        assert!(h.app.guides[0].vertical && (h.app.guides[0].pos - 150.0).abs() < 0.2, "{:?}", h.app.guides);
        let down = h.at(0.0, 80.0);
        h.drag(ruler_top(&h), egui::pos2(h.app.canvas_rect.unwrap().min.x + 400.0, down.y));
        assert_eq!(h.app.guides.len(), 2);
        assert!(!h.app.guides[1].vertical && (h.app.guides[1].pos - 80.0).abs() < 0.2);
        assert!(h.app.session.doc.shapes.is_empty(), "no stray shapes were created by the ruler drags");
        // Pull the vertical guide back onto the left ruler: it goes away.
        let on_guide = h.at(150.0, 0.0);
        h.drag(egui::pos2(on_guide.x, h.app.canvas_rect.unwrap().min.y + 300.0), ruler_left(&h));
        assert_eq!(h.app.guides.len(), 1);
        assert!(!h.app.guides[0].vertical);
    }

    #[test]
    fn guides_attract_snapping_and_can_be_cleared() {
        let mut h = Harness::new();
        h.app.guides.push(crate::canvas::Guide { vertical: true, pos: 200.0 });
        h.app.tool = Tool::Rectangle;
        h.drag(h.at(198.5, 50.0), h.at(260.0, 90.0));
        let b = h.app.session.doc.shapes.last().unwrap().world_bounds().unwrap();
        assert_eq!(b.x0, 200.0, "the left edge locks to the guide at x = 200 (the bed centre line is also there)");
        h.app.guides.clear();
        h.frame(vec![]);
        assert!(h.app.guides.is_empty());
    }

    #[test]
    fn rulers_can_be_hidden_and_then_the_whole_canvas_takes_drags() {
        let mut h = Harness::new();
        h.app.show_rulers = false;
        h.frame(vec![]);
        h.drag(ruler_left(&h), ruler_left(&h) + vec2(120.0, 0.0));
        assert!(h.app.guides.is_empty(), "no ruler, no guide");
        h.app.show_rulers = true;
        h.frame(vec![]);
    }
}

mod array_and_radius_tests {
    use super::*;

    #[test]
    fn the_rectangle_tool_uses_the_corner_radius() {
        let mut h = Harness::new();
        h.app.tool = Tool::Rectangle;
        h.app.corner_radius = 6.0;
        h.drag(h.at(50.0, 50.0), h.at(110.0, 100.0));
        let shape = &h.app.session.doc.shapes[0];
        assert_eq!(shape.name, "Rounded rectangle", "{:?}", h.app.status);
        assert!(!shape.world_polylines()[0].contains(Point::new(50.2, 50.2)), "the corner is cut away");
    }

    #[test]
    fn the_array_section_drives_both_array_commands() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"width": 10, "height": 10}));
        h.frame(vec![]); // renders the Array section
        h.app.array_columns = 2;
        h.app.array_rows = 2;
        h.app.run("arrange.grid", json!({"columns": h.app.array_columns, "rows": h.app.array_rows}));
        assert_eq!(h.app.session.doc.shapes.len(), 4);
        h.app.run("arrange.circular", json!({"count": h.app.array_count, "radius": h.app.array_radius}));
        assert_eq!(h.app.session.doc.shapes.len(), 4 * 6, "the whole grid is rotated as one selection");
    }
}

mod group_ui_tests {
    use super::*;

    #[test]
    fn ctrl_g_groups_and_ctrl_shift_g_ungroups_and_clicking_a_member_selects_all() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 30}));
        h.app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 30, "height": 30}));
        h.app.run("edit.select_all", serde_json::Value::Null);
        h.frame(vec![]);
        let key = |modifiers| Event::Key { key: egui::Key::G, physical_key: None, pressed: true, repeat: false, modifiers };
        h.frame(vec![key(Modifiers::COMMAND)]);
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        h.click(h.at(35.0, 35.0));
        assert_eq!(h.app.session.selection.len(), 2, "one click selects the whole group");
        h.frame(vec![key(Modifiers::COMMAND | Modifiers::SHIFT)]);
        h.click(h.at(300.0, 300.0));
        h.click(h.at(35.0, 35.0));
        assert_eq!(h.app.session.selection.len(), 1, "ungrouped shapes select alone");
    }

    #[test]
    fn the_layer_header_renames_and_reorders() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.app.run("layer.set", json!({"id": 0, "name": "Outline"}));
        h.app.run("layer.move", json!({"id": 0, "index": 3}));
        h.frame(vec![]);
        assert_eq!(h.app.session.doc.layers[3].name, "Outline");
    }
}

mod override_and_grid_tests {
    use super::*;

    #[test]
    fn the_override_section_and_test_grid_panel_render_and_drive_the_commands() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"width": 10, "height": 10}));
        h.frame(vec![]);
        h.app.run("shape.cut_override", json!({"speed": 300, "max_power": 70}));
        h.frame(vec![]);
        assert!(h.app.session.doc.shapes[0].cut.is_some());
        h.app.grid_cols = 3;
        h.app.grid_rows = 2;
        let params = json!({"cols": h.app.grid_cols, "rows": h.app.grid_rows, "layer": h.app.active_layer, "speed_min": h.app.grid_speed.0, "speed_max": h.app.grid_speed.1});
        assert!(h.app.run("generate.test_grid", params).is_some(), "{:?}", h.app.status);
        h.frame(vec![]);
        assert_eq!(h.app.session.doc.shapes.iter().filter(|s| s.cut.is_some()).count(), 1 + 6);
    }
}

mod shape_operation_tests {
    use super::*;

    fn two_squares(h: &mut Harness) {
        h.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40}));
        h.app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 40, "height": 40}));
        h.app.run("edit.select_all", serde_json::Value::Null);
        h.frame(vec![]);
    }

    #[test]
    fn menu_and_panel_defaults_drive_the_engine_commands() {
        let mut h = Harness::new();
        two_squares(&mut h);
        h.app.run("path.boolean", json!({"op": "union"}));
        assert_eq!(h.app.session.doc.shapes.len(), 1);
        h.app.offset_distance = 3.0;
        h.app.offset_join = "miter";
        let params = json!({ "distance": h.app.offset_distance, "join": h.app.offset_join, "keep_originals": h.app.offset_keep });
        h.app.run("path.offset", params);
        assert_eq!(h.app.session.doc.shapes.len(), 2, "originals are kept by default");
        h.frame(vec![]);
    }

    #[test]
    fn join_from_the_panel_closes_a_loop_of_lines() {
        let mut h = Harness::new();
        for (a, b) in [((10, 10), (50, 10)), ((50, 10), (50, 40)), ((50, 40), (10, 40)), ((10, 40), (10, 10))] {
            h.app.run("shape.line", json!({"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1}));
        }
        h.app.run("edit.select_all", serde_json::Value::Null);
        h.frame(vec![]);
        h.app.run("path.join", json!({}));
        assert_eq!(h.app.session.doc.shapes.len(), 1);
        assert!(h.app.session.doc.shapes[0].world_polylines()[0].closed);
    }

    #[test]
    fn a_boolean_with_one_shape_selected_reports_instead_of_crashing() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"width": 10, "height": 10}));
        assert!(h.app.run("path.boolean", json!({"op": "union"})).is_none());
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("at least two")), "{:?}", h.app.status);
    }
}

mod select_tests {
    use super::*;

    fn with_two_rects() -> Harness {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40}));
        h.app.run("shape.rectangle", json!({"x": 200, "y": 200, "width": 40, "height": 40}));
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        h
    }

    #[test]
    fn click_selects_and_empty_click_clears() {
        let mut h = with_two_rects();
        h.click(h.at(40.0, 40.0));
        assert_eq!(h.app.session.selection.len(), 1);
        h.click(h.at(120.0, 120.0));
        assert!(h.app.session.selection.is_empty());
    }

    #[test]
    fn dragging_a_shape_moves_it_as_one_undo_step() {
        let mut h = with_two_rects();
        let before = h.app.session.doc.shape(1).unwrap().world_bounds().unwrap();
        h.drag(h.at(40.0, 40.0), h.at(70.0, 55.0));
        let after = h.app.session.doc.shape(1).unwrap().world_bounds().unwrap();
        assert!((after.x0 - before.x0 - 30.0).abs() < 1.0 && (after.y0 - before.y0 - 15.0).abs() < 1.0, "{before:?} -> {after:?}");
        h.app.run("edit.undo", serde_json::Value::Null);
        assert_eq!(h.app.session.doc.shape(1).unwrap().world_bounds().unwrap(), before);
    }

    #[test]
    fn rubber_band_selects_fully_enclosed_shapes() {
        let mut h = with_two_rects();
        h.drag(h.at(10.0, 10.0), h.at(260.0, 260.0));
        assert_eq!(h.app.session.selection.len(), 2);
        h.drag(h.at(5.0, 5.0), h.at(70.0, 70.0));
        assert_eq!(h.app.session.selection, vec![1]);
    }

    #[test]
    fn dragging_a_corner_handle_resizes_in_one_step() {
        let mut h = with_two_rects();
        h.click(h.at(40.0, 40.0));
        h.drag(h.at(60.0, 60.0), h.at(100.0, 120.0)); // bottom-right handle of the 40x40 rect at (20,20)
        let b = h.app.session.doc.shape(1).unwrap().world_bounds().unwrap();
        assert!((b.x0 - 20.0).abs() < 1.0 && (b.width() - 80.0).abs() < 1.5 && (b.height() - 100.0).abs() < 1.5, "{b:?}");
        h.app.run("edit.undo", serde_json::Value::Null);
        let b = h.app.session.doc.shape(1).unwrap().world_bounds().unwrap();
        assert!((b.width() - 40.0).abs() < 1e-6);
    }

    #[test]
    fn delete_key_and_undo_shortcut_work() {
        let mut h = with_two_rects();
        h.click(h.at(40.0, 40.0));
        h.frame(vec![Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        assert_eq!(h.app.session.doc.shapes.len(), 1);
        h.frame(vec![Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }]);
        assert_eq!(h.app.session.doc.shapes.len(), 2);
    }
}

mod shortcut_tests {
    use super::*;
    use crate::shortcuts::{Action, Chord};

    fn press(key: egui::Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn a_rebound_shortcut_runs_the_command_and_the_old_one_stops() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
        h.app.run("edit.select_all", serde_json::Value::Null);
        let chord = Chord { command: true, shift: false, alt: true, key: egui::Key::J };
        h.app.shortcuts.set(Action::Duplicate, chord).unwrap();
        h.frame(vec![press(egui::Key::D, Modifiers::COMMAND)]);
        assert_eq!(h.app.session.doc.shapes.len(), 1, "the old chord no longer duplicates");
        h.frame(vec![press(egui::Key::J, Modifiers::COMMAND | Modifiers::ALT)]);
        assert_eq!(h.app.session.doc.shapes.len(), 2);
    }

    #[test]
    fn capturing_a_new_chord_binds_it_and_reports_conflicts() {
        let mut h = Harness::new();
        h.app.shortcuts_open = true;
        h.app.capturing = Some(Action::Group);
        h.frame(vec![press(egui::Key::A, Modifiers::COMMAND)]);
        assert!(h.app.capturing.is_none());
        assert!(h.app.shortcut_error.as_deref().is_some_and(|e| e.contains("Select all")), "{:?}", h.app.shortcut_error);
        assert_eq!(h.app.shortcuts.get(Action::Group).key, egui::Key::G);
        h.app.capturing = Some(Action::Group);
        h.frame(vec![press(egui::Key::K, Modifiers::COMMAND)]);
        assert_eq!(h.app.shortcuts.get(Action::Group).key, egui::Key::K);
        assert!(h.app.shortcut_error.is_none());
        h.app.capturing = Some(Action::Group);
        h.frame(vec![press(egui::Key::Escape, Modifiers::NONE)]);
        assert!(h.app.capturing.is_none());
        assert_eq!(h.app.shortcuts.get(Action::Group).key, egui::Key::K, "Escape cancels");
    }
}

mod accessibility_tests {
    use super::*;

    /// Every label the screen-reader tree carries after one frame.
    fn labels(h: &mut Harness) -> Vec<String> {
        h.ctx.enable_accesskit();
        h.time += 0.016;
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), SCREEN)), time: Some(h.time), ..Default::default() };
        let app = &mut h.app;
        let out = h.ctx.run_ui(input, |ui| app.ui(ui));
        let update = out.platform_output.accesskit_update.expect("a tree update");
        update.nodes.iter().filter_map(|(_, n)| n.label().map(str::to_string)).collect()
    }

    #[test]
    fn custom_drawn_controls_have_names_a_screen_reader_can_read() {
        let mut h = Harness::new();
        let first = labels(&mut h);
        assert!(first.iter().any(|l| l == "Design canvas"), "{first:?}");
        assert!(first.iter().any(|l| l.starts_with("Layer ")), "layer chips are named: {first:?}");
    }
}

mod machine_tests {
    use std::time::{Duration, Instant};

    use laserfoundry_device::JobState;
    use laserfoundry_doc::Origin;

    use super::*;

    /// Runs frames until `done` holds (the worker thread updates asynchronously).
    fn pump(h: &mut Harness, what: &str, mut done: impl FnMut(&Harness) -> bool) {
        let start = Instant::now();
        while !done(h) {
            h.frame(vec![]);
            assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}; status {:?}", h.app.status);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn position(h: &Harness) -> Option<(f64, f64)> {
        h.app.snap.as_ref().and_then(|s| s.status.as_ref()).and_then(|st| st.position).map(|(x, y, _)| (x, y))
    }

    fn connected() -> Harness {
        let mut h = Harness::new();
        h.app.connect();
        pump(&mut h, "the first status report", |h| position(h).is_some());
        h
    }

    #[test]
    fn the_head_shows_on_the_canvas_and_a_click_sends_it_there() {
        use crate::head_ui::{head_world, machine_to_world, world_to_machine};
        let mut h = connected();
        h.app.run("machine.set", json!({"origin": "bottom_left", "bed_height": 200}));
        let m = h.app.session.doc.machine.clone();
        assert_eq!(machine_to_world(&m, 10.0, 20.0), Point::new(10.0, 180.0));
        assert_eq!(world_to_machine(&m, Point::new(10.0, 180.0)), Some((10.0, 20.0)));
        for off in [Point::new(-1.0, 5.0), Point::new(5.0, 201.0), Point::new(f64::NAN, 5.0), Point::new(401.0, 5.0)] {
            assert_eq!(world_to_machine(&m, off), None, "{off:?}");
        }
        pump(&mut h, "status", |h| head_world(&h.app).is_some());

        h.app.move_head_mode = true;
        h.click(h.at(60.0, 150.0));
        pump(&mut h, "the head to arrive", |h| position(h).is_some_and(|(x, y)| (x - 60.0).abs() < 1e-6 && (y - 50.0).abs() < 1e-6));
        let p = head_world(&h.app).unwrap();
        assert!(p.distance(Point::new(60.0, 150.0)) < 1e-6, "{p:?}");

        h.click(h.at(60.0, 250.0)); // below the bed: refused, the head stays
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("outside")), "{:?}", h.app.status);
        h.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        assert!(!h.app.move_head_mode, "Escape leaves the mode");
        h.app.run("job.set_origin", json!({"start_from": "user_origin"}));
        assert_eq!(head_world(&h.app), None, "no mapping without absolute coordinates");
    }

    #[test]
    fn a_job_started_from_a_later_line_runs_on_the_simulator_and_finishes() {
        let mut h = connected();
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
        h.app.run("job.gcode", json!({}));
        let lines = h.app.session.last_gcode.as_ref().unwrap().lines().count();
        h.frame(vec![]); // draws the panel with the G-code section
        let v = h.app.run("job.resume", json!({"line": lines / 2, "include": true})).unwrap();
        h.app.start_job(v["gcode"].as_str().unwrap().to_string());
        pump(&mut h, "the resumed job", |h| matches!(h.app.snap.as_ref().map(|s| &s.job), Some(JobState::Finished(_))));
        assert!(matches!(h.app.snap.as_ref().map(|s| &s.job), Some(JobState::Finished(Ok(_)))), "{:?}", h.app.status);
    }

    #[test]
    fn controller_settings_are_read_edited_and_checked_against_the_document() {
        use crate::settings_ui::{Advice, advice};
        let mut h = connected();
        h.app.link_do(|l| l.request_settings());
        pump(&mut h, "the settings", |h| h.app.snap.as_ref().is_some_and(|s| s.settings.len() > 20));
        assert_eq!(h.app.snap.as_ref().and_then(|s| s.settings.get(&32)).map(String::as_str), Some("1"));

        let settings = h.app.snap.as_ref().map(|s| s.settings.clone()).unwrap();
        let machine = h.app.session.doc.machine.clone();
        assert_eq!(
            advice(&settings, &machine),
            vec![Advice::AccelDiffers(10.0), Advice::BedDiffers(200.0, 200.0)],
            "the simulator's travel is 200 mm, the document's bed 400, and its acceleration 10 mm/s² against none"
        );

        h.app.link_do(|l| l.write_setting(32, 0.0));
        pump(&mut h, "laser mode off", |h| h.app.snap.as_ref().and_then(|s| s.settings.get(&32)).is_some_and(|v| v == "0"));
        let settings = h.app.snap.as_ref().map(|s| s.settings.clone()).unwrap();
        assert!(advice(&settings, &machine).contains(&Advice::LaserModeOff));
        h.frame(vec![]); // the section draws with warnings present
    }

    #[test]
    fn the_machine_report_says_what_is_assumed_and_what_the_controller_reported() {
        let h = Harness::new();
        let none = crate::machine_report::report(&h.app);
        assert!(none.contains("Version: ") && none.contains("not connected"), "{none}");
        let mut h = connected();
        pump(&mut h, "the settings", |h| h.app.snap.as_ref().is_some_and(|s| s.settings.len() > 20));
        h.app.link_do(|l| l.send_line("G0 X1"));
        pump(&mut h, "the console", |h| h.app.snap.as_ref().is_some_and(|s| s.console.iter().any(|l| l.contains("G0 X1"))));
        let r = crate::machine_report::report(&h.app);
        assert!(r.contains("Connection: simulator  Connected: true"), "{r}");
        assert!(r.contains("$32=1") && r.contains("Controller settings ($$):") && r.contains("G0 X1"), "{r}");
        assert!(r.contains("Document machine: bed 400 x 400 mm"), "{r}");
        assert!(!r.contains("laserfoundry-ai") && !r.contains("token"), "nothing private");
    }

    #[test]
    fn a_job_waits_for_a_controller_that_is_idle() {
        use laserfoundry_device::{MachineState, Status};
        let mut h = connected();
        for (state, word) in [
            (MachineState::Alarm, "alarm"),
            (MachineState::Hold, "holding"),
            (MachineState::Door, "door"),
            (MachineState::Home, "homing"),
            (MachineState::Run, "moving"),
            (MachineState::Check, "check mode"),
        ] {
            h.app.snap.as_mut().unwrap().status = Some(Status { state, position: None, position_is_work: false });
            h.app.start_job("G21\n".into());
            assert!(matches!(&h.app.status, Some((m, true)) if m.contains(word)), "{state:?}: {:?}", h.app.status);
            assert!(h.app.job_program.is_none() && h.app.pending_start.is_none());
        }
        h.app.snap.as_mut().unwrap().status = Some(Status { state: MachineState::Idle, position: None, position_is_work: false });
        h.app.start_job("G21\n".into());
        assert!(h.app.job_program.is_some(), "idle starts it");
    }

    #[test]
    fn a_job_is_refused_while_the_controller_is_not_in_laser_mode() {
        let mut h = connected();
        // The settings are fetched by themselves once the controller answers.
        pump(&mut h, "the settings", |h| h.app.snap.as_ref().is_some_and(|s| s.settings.len() > 20));
        h.app.link_do(|l| l.write_setting(32, 0.0));
        pump(&mut h, "laser mode off", |h| h.app.snap.as_ref().and_then(|s| s.settings.get(&32)).is_some_and(|v| v == "0"));
        h.app.start_job("G21\n".into());
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("Laser mode")), "{:?}", h.app.status);
        assert!(h.app.job_program.is_none() && h.app.pending_start.is_none());

        h.app.link_do(|l| l.write_setting(32, 1.0));
        pump(&mut h, "laser mode on", |h| h.app.snap.as_ref().and_then(|s| s.settings.get(&32)).is_some_and(|v| v == "1"));
        h.app.start_job("G21\n".into());
        assert!(h.app.job_program.is_some());
    }

    #[test]
    fn advice_ignores_missing_and_hostile_values() {
        use std::collections::BTreeMap;

        use crate::settings_ui::{Advice, advice};
        let machine = laserfoundry_doc::Machine::default();
        assert!(advice(&BTreeMap::new(), &machine).is_empty());
        let junk: BTreeMap<u32, String> = [(30, "NaN"), (32, "x"), (130, "inf"), (131, "-5")].into_iter().map(|(k, v)| (k, v.to_string())).collect();
        assert!(advice(&junk, &machine).is_empty());
        let huge: BTreeMap<u32, String> = [(30, "1e30")].into_iter().map(|(k, v)| (k, v.to_string())).collect();
        assert!(advice(&huge, &machine).is_empty(), "an absurd S max is not offered");
        let s: BTreeMap<u32, String> = [(30, "255")].into_iter().map(|(k, v)| (k, v.to_string())).collect();
        assert_eq!(advice(&s, &machine), vec![Advice::SMaxDiffers(255)]);
    }

    #[test]
    fn simulator_job_runs_to_completion_and_reports() {
        let mut h = connected();
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
        let program = h.app.run("job.gcode", json!({"include": true})).and_then(|v| v["gcode"].as_str().map(String::from)).unwrap();
        h.app.start_job(program);
        pump(&mut h, "the job", |h| matches!(h.app.snap.as_ref().map(|s| &s.job), Some(JobState::Finished(_))));
        h.frame(vec![]);
        assert!(matches!(&h.app.status, Some((m, false)) if m.starts_with("Job finished")), "{:?}", h.app.status);
    }

    #[test]
    fn jog_directions_follow_the_machine_origin() {
        assert_eq!(panels::machine_delta(Origin::TopLeft, 0.0, -5.0), (0.0, -5.0));
        assert_eq!(panels::machine_delta(Origin::BottomLeft, 0.0, -5.0), (0.0, 5.0), "screen-up is +Y on a bottom-left machine");
        assert_eq!(panels::machine_delta(Origin::BottomLeft, 3.0, 0.0), (3.0, 0.0));
    }

    #[test]
    fn a_jog_through_the_link_moves_the_simulated_head() {
        let mut h = connected();
        h.app.link_do(|l| l.jog(0.0, 10.0, 3000.0));
        pump(&mut h, "the jog", |h| position(h) == Some((0.0, 10.0)));
    }

    #[test]
    fn a_job_placed_from_the_head_waits_for_a_confirmation() {
        let mut h = connected();
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
        h.app.run("job.set_origin", json!({"start_from": "current_position"}));
        h.app.start_job("G21\n".into());
        assert!(h.app.pending_start.is_some(), "nothing is sent before the person agrees");
        assert!(h.app.job_program.is_none());
        h.frame(vec![]);
        assert!(h.app.has_modal());

        // Cancel drops it; asking again and confirming sends it.
        h.app.pending_start = None;
        h.app.start_job("G21\n".into());
        let program = h.app.pending_start.take().unwrap();
        h.app.begin_job(program);
        assert!(h.app.job_program.is_some());

        // With absolute coordinates the bed is checked by the program itself: no question.
        h.app.job_program = None;
        h.app.run("job.set_origin", json!({"start_from": "absolute"}));
        h.app.start_job("G21\n".into());
        assert!(h.app.pending_start.is_none());
        assert!(h.app.job_program.is_some());
    }

    #[test]
    fn starting_without_a_connection_explains_what_to_do() {
        let mut h = Harness::new();
        h.app.start_job("G21\n".into());
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("Connect")), "{:?}", h.app.status);
    }

    #[test]
    fn serial_target_without_a_platform_reports_instead_of_crashing() {
        let mut h = Harness::new();
        h.app.target = Target::Serial("/dev/ttyUSB0".into());
        h.app.connect();
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("Could not open")), "{:?}", h.app.status);
        assert!(h.app.link.is_none());
    }

    #[test]
    fn disconnect_mid_job_cancels_and_clears_state() {
        let mut h = connected();
        let program: String = (0..3000).map(|i| format!("G1 X{} Y0 F100\n", i % 100)).collect();
        h.app.start_job(program);
        pump(&mut h, "the job to start", |h| h.app.job_active());
        h.app.disconnect();
        assert!(h.app.link.is_none() && h.app.snap.is_none());
        h.frame(vec![]);
    }

    #[test]
    fn job_origin_controls_drive_the_engine_command() {
        let mut h = Harness::new();
        h.app.run("job.set_origin", json!({"start_from": "user_origin", "anchor": "center"}));
        h.frame(vec![]);
        assert_eq!(h.app.session.doc.job_origin.anchor, laserfoundry_doc::Anchor::Center);
    }

    #[test]
    fn the_laser_panel_renders_connected_and_disconnected() {
        let mut h = connected();
        for _ in 0..3 {
            h.frame(vec![]);
        }
        h.app.disconnect();
        h.frame(vec![]);
    }
}

mod timeline_ui_tests {
    use laserfoundry_doc::LayerId;
    use laserfoundry_toolpath::Moment;

    use super::*;
    use crate::geometry::PreviewSegment;
    use crate::timeline_ui::{Reveal, head_position, reveal};

    #[test]
    fn the_heat_ramp_runs_cold_to_hot_and_survives_bad_values() {
        use crate::timeline_ui::{fraction, heat};
        let (cold, hot) = (heat(0.0), heat(1.0));
        assert!(cold.b() > cold.r() && hot.r() > hot.b(), "{cold:?} {hot:?}");
        assert_eq!(heat(-5.0), cold);
        assert_eq!(heat(7.0), hot);
        assert_eq!(heat(f32::NAN), cold);
        let mid = heat(0.5);
        assert!(mid != cold && mid != hot);
        assert_eq!(fraction(5.0, 0.0, 10.0), 0.5);
        assert_eq!(fraction(20.0, 0.0, 10.0), 1.0);
        assert_eq!(fraction(5.0, 3.0, 3.0), 0.5, "an empty range is the middle");
        assert_eq!(fraction(f32::NAN, 0.0, 1.0), 0.5);
    }

    #[test]
    fn every_colour_mode_draws_a_preview_with_several_powers_and_speeds() {
        use crate::timeline_ui::PreviewColors;
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20}));
        h.app.run("shape.ellipse", json!({"cx": 100, "cy": 40, "rx": 15, "ry": 10, "layer": 2}));
        h.app.run("layer.set", json!({"id": 2, "speed": 300, "max_power": 90}));
        h.app.show_preview = true;
        for mode in [PreviewColors::Layer, PreviewColors::Power, PreviewColors::Speed] {
            h.app.preview_colors = mode;
            h.frame(vec![]);
        }
        let (lo, hi) = h.app.preview.speed_range;
        assert!(lo < hi, "two different feeds were seen: {lo} {hi}");
        assert!(h.app.preview.power_range.0 < h.app.preview.power_range.1);
    }

    #[test]
    fn segments_are_revealed_by_operation_pass_and_time() {
        let seg = segment(1, 2.0, 4.0);
        assert_eq!(reveal(&seg, None), Reveal::Full, "no playhead shows the whole job");
        assert_eq!(reveal(&seg, Some(moment(0, 0, 9.0))), Reveal::Hidden, "an earlier operation has not reached it");
        assert_eq!(reveal(&seg, Some(moment(2, 0, 0.0))), Reveal::Full, "a later operation has finished it");
        assert_eq!(reveal(&seg, Some(moment(1, 1, 0.0))), Reveal::Full, "a second pass starts over a finished first one");
        assert_eq!(reveal(&seg, Some(moment(1, 0, 1.0))), Reveal::Hidden);
        assert_eq!(reveal(&seg, Some(moment(1, 0, 3.0))), Reveal::Partial(0.5));
        assert_eq!(reveal(&seg, Some(moment(1, 0, 4.0))), Reveal::Full);
    }

    #[test]
    fn the_head_sits_inside_the_segment_being_cut() {
        let segs = [segment(0, 0.0, 2.0), segment(0, 2.0, 4.0)];
        let p = head_position(&segs, moment(0, 0, 1.0)).unwrap();
        assert!(p.distance(Point::new(2.5, 0.0)) < 1e-6, "{p:?}");
        let end = head_position(&segs, moment(0, 0, 4.0)).unwrap();
        assert!(end.distance(Point::new(10.0, 0.0)) < 1e-6, "{end:?}");
        assert_eq!(head_position(&segs, moment(3, 0, 1.0)), None, "no segments of that operation");
    }

    #[test]
    fn playing_runs_to_the_end_and_stops_and_the_scrubber_shows_the_head() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 30}));
        h.app.show_preview = true;
        h.frame(vec![]);
        let total = h.app.preview.timeline.total;
        assert!(total > 0.0);
        assert!(h.app.preview.segments.iter().all(|s| s.t1 >= s.t0 && f64::from(s.t1) <= total + 1e-3));
        assert_eq!(h.app.player.position, None, "the preview starts by showing the whole job");

        h.app.player.speed = 4; // 600x
        h.app.player.position = Some(0.0);
        h.app.player.playing = true;
        let mut frames = 0;
        while h.app.player.playing && frames < 5000 {
            h.frame(vec![]);
            frames += 1;
        }
        assert!(!h.app.player.playing, "playback stops by itself");
        assert_eq!(h.app.player.position, Some(total));

        h.app.player.position = Some(total * 0.5);
        h.frame(vec![]);
        let now = h.app.preview.timeline.at(total * 0.5).unwrap();
        assert!(head_position(&h.app.preview.segments, now).is_some());
    }

    #[test]
    fn an_empty_job_has_no_timeline_and_does_not_break_the_frame() {
        let mut h = Harness::new();
        h.app.show_preview = true;
        h.app.player.position = Some(5.0);
        h.app.player.playing = true;
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.app.preview.timeline.ops.is_empty());
    }

    #[test]
    fn layer_times_in_the_estimate_name_each_layer() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 30}));
        h.app.run("shape.ellipse", json!({"cx": 100, "cy": 100, "rx": 20, "ry": 10, "layer": 2}));
        let v = h.app.run("job.estimate", serde_json::Value::Null).unwrap();
        assert_eq!(v["layers"].as_array().map(Vec::len), Some(2));
    }

    fn segment(op: u32, t0: f32, t1: f32) -> PreviewSegment {
        let layer: LayerId = 0;
        PreviewSegment {
            a: Point::new(f64::from(t0) * 2.5, 0.0),
            b: Point::new(f64::from(t1) * 2.5, 0.0),
            layer,
            power: 50.0,
            speed: 1000.0,
            op,
            t0,
            t1,
        }
    }

    fn moment(op: usize, pass: u32, into_pass: f64) -> Moment {
        Moment { op, pass, into_pass }
    }
}

mod recent_files_tests {
    use super::*;
    use crate::PendingAction;
    use crate::recent::{MAX_RECENT, Recent};

    #[test]
    fn the_list_is_most_recent_first_without_duplicates_and_capped() {
        let mut r = Recent::default();
        for i in 0..15 {
            r.push(format!("/p/{i}.laserfoundry").into());
        }
        assert_eq!(r.files.len(), MAX_RECENT);
        assert_eq!(r.files[0], std::path::PathBuf::from("/p/14.laserfoundry"));
        r.push("/p/10.laserfoundry".into());
        assert_eq!(r.files[0], std::path::PathBuf::from("/p/10.laserfoundry"));
        assert_eq!(r.files.iter().filter(|p| p.ends_with("10.laserfoundry")).count(), 1);
        r.remove(std::path::Path::new("/p/10.laserfoundry"));
        assert_eq!(r.files.len(), MAX_RECENT - 1);
    }

    #[test]
    fn saving_and_loading_round_trips_and_hostile_files_give_an_empty_list() {
        let dir = temp_dir("recent-file");
        let file = dir.join("deep").join("recent.json");
        let mut r = Recent::default();
        r.push("/a.laserfoundry".into());
        r.push("/b.laserfoundry".into());
        r.save(&file);
        assert_eq!(Recent::load(&file), r);
        for (name, bytes) in [("garbage", b"\x00\xff not json".to_vec()), ("object", br#"{"a": 1}"#.to_vec()), ("huge", vec![b' '; 300 * 1024])] {
            let bad = dir.join(name);
            std::fs::write(&bad, bytes).unwrap();
            assert!(Recent::load(&bad).files.is_empty(), "{name}");
        }
        std::fs::write(dir.join("mixed"), br#"["/x.laserfoundry", 5, null, "", "/y.laserfoundry"]"#).unwrap();
        assert_eq!(Recent::load(&dir.join("mixed")).files.len(), 2, "non-strings and empty entries are skipped");
        assert!(Recent::load(&dir.join("missing")).files.is_empty());
    }

    #[test]
    fn saving_and_opening_projects_updates_the_list_and_a_missing_file_is_forgotten() {
        let dir = temp_dir("recent-app");
        let list = dir.join("recent.json");
        let mut h = harness_with(RecoveryPlatform { file: dir.join("r.laserfoundry"), open_answer: None, recent: Some(list.clone()) });
        assert!(h.app.recent.files.is_empty());
        h.app.run("shape.rectangle", json!({"width": 20, "height": 10}));
        let project = dir.join("one.laserfoundry");
        h.app.run("file.save", json!({"path": project}));
        h.frame(vec![]);
        assert_eq!(h.app.recent.files, vec![project.clone()]);
        assert_eq!(Recent::load(&list).files, vec![project.clone()], "written to disk");

        h.app.request(PendingAction::OpenPath(dir.join("gone.laserfoundry")));
        h.frame(vec![]);
        assert!(h.app.status.as_ref().is_some_and(|s| s.1), "the failure is reported");

        h.app.recent.push(dir.join("gone.laserfoundry"));
        h.app.request(PendingAction::OpenPath(dir.join("gone.laserfoundry")));
        assert_eq!(h.app.recent.files, vec![project.clone()], "an unreadable entry is dropped from the list");

        h.app.run("file.new", serde_json::Value::Null);
        h.app.request(PendingAction::OpenPath(project.clone()));
        h.frame(vec![]);
        assert_eq!(h.app.session.path.as_deref(), Some(project.as_path()));
        assert_eq!(h.app.session.doc.shapes.len(), 1);
    }

    #[test]
    fn opening_a_recent_project_over_unsaved_work_asks_first() {
        let dir = temp_dir("recent-ask");
        let mut h = harness_with(RecoveryPlatform { file: dir.join("r.laserfoundry"), open_answer: None, recent: None });
        h.app.run("shape.rectangle", json!({"width": 20, "height": 10}));
        h.app.request(PendingAction::OpenPath(dir.join("x.laserfoundry")));
        assert_eq!(h.app.pending_action, Some(PendingAction::OpenPath(dir.join("x.laserfoundry"))), "the prompt is raised, nothing is opened yet");
        assert_eq!(h.app.session.doc.shapes.len(), 1);
    }
}

mod preset_ui_tests {
    use super::*;

    #[test]
    fn the_presets_section_renders_with_and_without_presets() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.app.run("layer.set", json!({"id": 0, "speed": 333}));
        h.app.preset_name = "Test card".into();
        h.app.run("preset.save", json!({"name": h.app.preset_name.clone(), "layer": 0}));
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.app.session.doc.presets.len(), 1);
    }
}

mod create_by_numbers_tests {
    use super::*;

    #[test]
    fn typed_sizes_make_exact_shapes_on_the_active_layer() {
        let mut h = Harness::new();
        h.app.active_layer = 3;
        h.app.new_shape = [20.0, 30.0, 40.0, 10.0];
        h.frame(vec![]);
        h.frame(vec![]); // renders the section: nothing is selected
        let [x, y, w, hh] = h.app.new_shape;
        h.app.run("shape.rectangle", json!({"x": x, "y": y, "width": w, "height": hh, "layer": h.app.active_layer}));
        let b = h.app.session.doc.shapes[0].world_bounds().unwrap();
        assert!((b.x0 - 20.0).abs() < 1e-6 && (b.y1 - 40.0).abs() < 1e-6 && (b.x1 - 60.0).abs() < 1e-6, "{b:?}");
        assert_eq!(h.app.session.doc.shapes[0].layer, 3);
    }

    #[test]
    fn a_new_rectangle_stops_a_gap_short_of_a_neighbour_that_repeats_an_existing_gap() {
        let mut h = Harness::new();
        for x in [10.0, 40.0, 150.0] {
            h.app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20}));
        }
        h.app.run("edit.select", json!({"ids": []}));
        h.app.snapping = crate::snap::SnapSettings { grid: None, objects: true, ..Default::default() };
        h.app.tool = Tool::Rectangle;
        h.frame(vec![]);
        // The first two are 10 mm apart; dragging to 141 leaves 9 mm before the one at 150, so it snaps to 140.
        h.drag(h.at(100.0, 10.0), h.at(141.0, 30.0));
        let b = h.app.session.doc.shapes[3].world_bounds().unwrap();
        assert!((b.x1 - 140.0).abs() < 1e-6, "{b:?}");
        // With object snapping off the drag is taken as is.
        h.app.snapping.objects = false;
        h.app.tool = Tool::Rectangle;
        h.drag(h.at(100.0, 50.0), h.at(141.0, 70.0));
        let b = h.app.session.doc.shapes[4].world_bounds().unwrap();
        assert!((b.x1 - 141.0).abs() < 0.5, "{b:?}");
    }

    #[test]
    fn snapping_reaches_path_vertices_that_are_not_on_the_bounding_box() {
        let mut h = Harness::new();
        h.app.run("path.create", json!({"nodes": [{"x": 100.0, "y": 100.0}, {"x": 180.0, "y": 100.0}, {"x": 120.0, "y": 140.0}], "closed": true}));
        h.app.run("edit.select", json!({"ids": []}));
        h.app.snapping = crate::snap::SnapSettings { grid: None, objects: true, ..Default::default() };
        h.app.tool = Tool::Rectangle;
        h.frame(vec![]);
        // x = 120 is only the triangle's lower vertex: its box edges are 100, 140 and 180.
        h.drag(h.at(121.0, 60.0), h.at(200.0, 80.0));
        let b = h.app.session.doc.shapes[1].world_bounds().unwrap();
        assert!((b.x0 - 120.0).abs() < 1e-6, "{b:?}");
    }
}

mod tablet_tests {
    use egui::{TouchDeviceId, TouchId, TouchPhase};

    use super::*;
    use crate::tablet::TabletMode;

    fn touch(id: u64, phase: TouchPhase, pos: egui::Pos2) -> Event {
        Event::Touch { device_id: TouchDeviceId(0), id: TouchId(id), phase, pos, force: None }
    }

    /// Two fingers start at `a` and `b`, move together by `by`, and stay down.
    fn two_fingers(h: &mut Harness, a: egui::Pos2, b: egui::Pos2, a2: egui::Pos2, b2: egui::Pos2) {
        // Real integrations send a pointer position along with touches; egui starts a gesture from it.
        h.frame(vec![Event::PointerMoved(a), touch(1, TouchPhase::Start, a), touch(2, TouchPhase::Start, b)]);
        h.frame(vec![]);
        for step in 1..=4 {
            let t = step as f32 / 4.0;
            h.frame(vec![touch(1, TouchPhase::Move, a + (a2 - a) * t), touch(2, TouchPhase::Move, b + (b2 - b) * t)]);
        }
    }

    #[test]
    fn the_first_touch_turns_automatic_tablet_mode_on_and_the_setting_can_force_it() {
        let mut h = Harness::new();
        assert!(!h.app.tablet_active());
        assert_eq!(h.app.hit_px(), crate::tablet::HIT_PX_MOUSE);
        h.frame(vec![touch(1, TouchPhase::Start, pos2(500.0, 400.0))]);
        h.frame(vec![touch(1, TouchPhase::End, pos2(500.0, 400.0))]);
        assert!(h.app.tablet_active());
        assert_eq!(h.app.hit_px(), crate::tablet::HIT_PX_TOUCH);
        assert_eq!(h.ctx.zoom_factor(), crate::tablet::UI_ZOOM);
        h.app.tablet = TabletMode::Off;
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(!h.app.tablet_active());
        assert_eq!(h.ctx.zoom_factor(), 1.0, "turning it off restores the normal size");
        h.app.sticky_shift = true;
        h.frame(vec![]);
        assert!(!h.app.sticky_shift, "the sticky Shift goes with the mode");

        let mut forced = Harness::new();
        forced.app.tablet = TabletMode::On;
        forced.frame(vec![]);
        assert!(forced.app.tablet_active() && !forced.app.touch_seen);
        assert!(crate::tablet::starts_folded(820.0) && !crate::tablet::starts_folded(1366.0));
    }

    #[test]
    fn two_fingers_pan_and_zoom_the_view_and_edit_nothing() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 20, "height": 20}));
        h.app.run("edit.select", json!({"ids": []}));
        h.app.tablet = TabletMode::On;
        h.settle();
        let before = h.app.view.unwrap();
        // Both fingers slide right and down by (40, 20).
        two_fingers(&mut h, pos2(500.0, 300.0), pos2(600.0, 300.0), pos2(540.0, 320.0), pos2(640.0, 320.0));
        let after = h.app.view.unwrap();
        assert!(
            (after.origin.x - before.origin.x - 40.0).abs() < 1.0 && (after.origin.y - before.origin.y - 20.0).abs() < 1.0,
            "{before:?} {after:?}"
        );
        assert!((after.zoom - before.zoom).abs() < 0.01, "no pinch, no zoom");
        h.frame(vec![touch(1, TouchPhase::End, pos2(540.0, 320.0)), touch(2, TouchPhase::End, pos2(640.0, 320.0))]);

        // Spreading the fingers zooms in.
        let before = h.app.view.unwrap();
        two_fingers(&mut h, pos2(500.0, 300.0), pos2(600.0, 300.0), pos2(450.0, 300.0), pos2(650.0, 300.0));
        assert!(h.app.view.unwrap().zoom > before.zoom * 1.2, "{before:?} {:?}", h.app.view);
        h.frame(vec![touch(1, TouchPhase::End, pos2(450.0, 300.0)), touch(2, TouchPhase::End, pos2(650.0, 300.0))]);
        assert_eq!(h.app.session.doc.shapes.len(), 1, "no shape was made or lost");
        assert!(h.app.session.selection.is_empty());
    }

    #[test]
    fn a_pointer_drag_that_two_fingers_join_is_dropped_not_applied() {
        let mut h = Harness::new();
        h.app.tool = Tool::Rectangle;
        h.frame(vec![]);
        let start = pos2(400.0, 300.0);
        h.frame(vec![Event::PointerMoved(start)]);
        h.frame(vec![Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        h.frame(vec![Event::PointerMoved(pos2(450.0, 340.0))]);
        two_fingers(&mut h, pos2(500.0, 300.0), pos2(600.0, 300.0), pos2(510.0, 300.0), pos2(610.0, 300.0));
        assert!(matches!(h.app.drag, crate::canvas::Drag::None), "{:?}", h.app.drag);
        h.frame(vec![Event::PointerButton { pos: pos2(450.0, 340.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        h.frame(vec![]);
        assert_eq!(h.app.session.doc.shapes.len(), 0, "the half-made rectangle was not created");
    }

    #[test]
    fn the_pan_tool_drags_the_view_with_one_finger() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 20, "height": 20}));
        h.app.run("edit.select", json!({"ids": []}));
        h.app.tool = Tool::Hand;
        h.frame(vec![]);
        h.frame(vec![]);
        let before = h.app.view.unwrap();
        // Dragging across the rectangle moves the view, not the rectangle, and selects nothing.
        let from = h.at(60.0, 60.0);
        h.drag(from, from + vec2(60.0, 35.0));
        let after = h.app.view.unwrap();
        assert!(
            (after.origin.x - before.origin.x - 60.0).abs() < 2.0 && (after.origin.y - before.origin.y - 35.0).abs() < 2.0,
            "{before:?} {after:?}"
        );
        let b = h.app.session.doc.shapes[0].world_bounds().unwrap();
        assert!((b.x0 - 50.0).abs() < 1e-9 && (b.y0 - 50.0).abs() < 1e-9, "{b:?}");
        assert!(h.app.session.selection.is_empty());
    }

    #[test]
    fn the_sticky_shift_keeps_a_move_on_one_axis() {
        let mut h = Harness::new();
        h.app.tablet = TabletMode::On;
        h.app.snapping = crate::snap::SnapSettings { grid: None, objects: false, ..Default::default() };
        let id = h.app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        h.app.run("edit.select", json!({"ids": [id]}));
        h.settle();
        h.app.sticky_shift = true;
        h.frame(vec![]);
        // Drag diagonally: mostly right, a little down.
        h.drag(h.at(60.0, 60.0), h.at(90.0, 66.0));
        let b = h.app.session.doc.shape(id).unwrap().world_bounds().unwrap();
        assert!((b.x0 - 80.0).abs() < 1.0, "moved right: {b:?}");
        assert!((b.y0 - 50.0).abs() < 1e-6, "but not down: {b:?}");
    }

    #[test]
    fn a_fingertip_grabs_what_a_mouse_pointer_misses() {
        let click_near_edge = |mode: TabletMode| {
            let mut h = Harness::new();
            h.app.tablet = mode;
            h.app.snapping = crate::snap::SnapSettings { grid: None, objects: false, ..Default::default() };
            h.app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 40, "height": 40}));
            h.app.run("edit.select", json!({"ids": []}));
            h.settle();
            // Eleven screen points outside the right edge, level with its middle.
            let edge = h.at(80.0, 60.0);
            h.click(edge + vec2(11.0, 0.0));
            h.app.session.selection.len()
        };
        assert_eq!(click_near_edge(TabletMode::Off), 0, "outside the 6 point reach of a pointer");
        assert_eq!(click_near_edge(TabletMode::On), 1, "inside the 16 point reach of a finger");
    }

    #[test]
    fn the_action_bar_buttons_work_and_the_inspector_folds_away() {
        let mut h = Harness::new();
        h.app.tablet = TabletMode::On;
        let id = h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.app.show_inspector);
        // The bar is real: its Undo button undoes the rectangle.
        let undo = h.app.button_rects["Undo"].center();
        h.click(undo);
        assert!(h.app.session.doc.shape(id).is_none(), "undone from the action bar");
        let panel = h.app.button_rects["Panel"].center();
        h.click(panel);
        assert!(!h.app.show_inspector, "the Panel button folds the inspector away");
        let panel = h.app.button_rects["Panel"].center();
        h.click(panel);
        assert!(h.app.show_inspector);
    }
}

mod context_menu_tests {
    use super::*;

    fn secondary_click(h: &mut Harness, at: egui::Pos2) {
        h.frame(vec![Event::PointerMoved(at)]);
        h.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE }]);
        h.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE }]);
        h.frame(vec![]);
        h.frame(vec![]);
    }

    #[test]
    fn a_right_click_selects_what_is_under_it_and_the_menu_edits_it() {
        let mut h = Harness::new();
        let id = h.app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
        h.app.run("shape.rectangle", json!({"x": 150, "y": 50, "width": 30, "height": 30}));
        h.app.run("edit.select", json!({"ids": []}));
        h.settle();
        // On the first rectangle's outline.
        {
            let at = h.at(50.0, 65.0);
            secondary_click(&mut h, at);
        }
        assert_eq!(h.app.session.selection, vec![id], "the shape under the pointer is selected first");
        assert!(h.app.menu_point.is_some());
        let duplicate = h.app.button_rects["Duplicate"].center();
        h.click(duplicate);
        assert_eq!(h.app.session.doc.shapes.len(), 3, "Duplicate from the menu");
        // Empty canvas: the selection is left alone, and Delete (still enabled) acts on it.
        let before = h.app.session.selection.clone();
        {
            let at = h.at(300.0, 300.0);
            secondary_click(&mut h, at);
        }
        assert_eq!(h.app.session.selection, before);
        let delete = h.app.button_rects["Delete"].center();
        h.click(delete);
        assert_eq!(h.app.session.doc.shapes.len(), 2);
    }

    #[test]
    fn a_plain_rectangles_corner_can_be_tracked_from_the_menu() {
        let mut h = Harness::new();
        h.app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 30, "height": 30}));
        h.app.run("edit.select", json!({"ids": []}));
        h.settle();
        {
            let at = h.at(80.0, 50.0);
            secondary_click(&mut h, at);
        }
        let track = h.app.button_rects["Track this point"].center();
        h.click(track);
        assert_eq!(h.app.tracked.len(), 1, "{:?} menu point {:?}", h.app.tracked, h.app.menu_point);
    }

    #[test]
    fn the_menu_acquires_and_forgets_a_tracking_point_and_adds_guides_without_hovering() {
        let mut h = Harness::new();
        h.app.snapping = crate::snap::SnapSettings { grid: None, objects: true, ..Default::default() };
        h.app.run("path.create", json!({"nodes": [{"x": 100.0, "y": 100.0}, {"x": 180.0, "y": 100.0}, {"x": 120.0, "y": 140.0}], "closed": true}));
        h.app.run("edit.select", json!({"ids": []}));
        h.settle();
        assert!(h.app.tracked.is_empty());
        {
            let at = h.at(100.0, 100.0);
            secondary_click(&mut h, at);
        }
        let track = h.app.button_rects["Track this point"].center();
        h.click(track);
        assert_eq!(h.app.tracked.len(), 1, "the vertex was acquired");
        // Again on the same vertex: the item now forgets it.
        {
            let at = h.at(100.0, 100.0);
            secondary_click(&mut h, at);
        }
        let forget = h.app.button_rects["Stop tracking this point"].center();
        h.click(forget);
        assert!(h.app.tracked.is_empty());
        // A guide through the pressed point, from the same menu.
        {
            let at = h.at(100.0, 100.0);
            secondary_click(&mut h, at);
        }
        let guide = h.app.button_rects["Vertical guide here"].center();
        h.click(guide);
        assert_eq!(h.app.guides.len(), 1);
        assert!(h.app.guides[0].vertical && (h.app.guides[0].pos - 100.0).abs() < 1.0, "{:?}", h.app.guides);
        // Away from any point the item is disabled, and nothing is acquired.
        {
            let at = h.at(300.0, 250.0);
            secondary_click(&mut h, at);
        }
        let disabled = h.app.button_rects["Track this point"].center();
        h.click(disabled);
        assert!(h.app.tracked.is_empty());
    }
}

mod selection_scope_tests {
    use super::*;

    #[test]
    fn the_preview_follows_the_selection_when_only_it_is_cut() {
        let mut h = Harness::new();
        let a = h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap()["id"].as_u64().unwrap();
        h.app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 20, "height": 10}));
        h.app.show_preview = true;
        h.frame(vec![]);
        let all = h.app.preview.segments.len();
        h.app.run("job.set_scope", json!({"selection_only": true}));
        h.app.run("edit.select", json!({"ids": [a]}));
        h.frame(vec![]);
        let some = h.app.preview.segments.len();
        assert!(some > 0 && some < all, "{some} of {all}");
        h.app.run("edit.select", json!({"ids": []}));
        h.frame(vec![]);
        assert_eq!(h.app.preview.segments.len(), 0, "no selection, nothing to preview");
    }
}

mod reconnect_tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use laserfoundry_device::{Link, SimulatedGrbl, Transport};

    use super::*;

    /// A serial port that fails `fail_times` times and then opens a simulated controller.
    struct FlakyPort {
        fail_times: u32,
        opens: Rc<Cell<u32>>,
    }

    impl Platform for FlakyPort {
        fn pick_open(&self, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn serial_ports(&self) -> Vec<String> {
            vec!["ttyUSB0".into()]
        }

        fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
            self.opens.set(self.opens.get() + 1);
            if self.opens.get() <= self.fail_times { Err("no such device".into()) } else { Ok(Box::new(SimulatedGrbl::new())) }
        }
    }

    struct Unplugged;

    impl Transport for Unplugged {
        fn write_all(&mut self, _: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::other("unplugged"))
        }

        fn read_line(&mut self, _: Duration) -> std::io::Result<Option<String>> {
            Err(std::io::Error::other("unplugged"))
        }
    }

    fn harness(fail_times: u32) -> (Harness, Rc<Cell<u32>>) {
        let opens = Rc::new(Cell::new(0));
        let mut h = Harness {
            ctx: egui::Context::default(),
            app: LaserFoundryApp::new(Session::new(), Box::new(FlakyPort { fail_times, opens: Rc::clone(&opens) })),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
        };
        h.app.reconnect_wait = Duration::ZERO;
        h.app.target = Target::Serial("ttyUSB0".into());
        h.app.link = Some(Link::connect(Box::new(Unplugged)));
        (h, opens)
    }

    fn run_until(h: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
        let start = Instant::now();
        while !done(h) {
            h.frame(vec![]);
            assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}: {:?}", h.app.status);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_lost_connection_is_retried_until_it_comes_back_and_nothing_resumes() {
        let (mut h, opens) = harness(2);
        run_until(&mut h, "the loss", |h| h.app.reconnect.is_some());
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("trying to reconnect")), "{:?}", h.app.status);
        run_until(&mut h, "the reconnect", |h| h.app.link.is_some() && h.app.reconnect.is_none());
        assert_eq!(opens.get(), 3, "two failures, then success");
        assert!(matches!(&h.app.status, Some((m, false)) if m.contains("Reconnected") && m.contains("Nothing is running")), "{:?}", h.app.status);
        run_until(&mut h, "a status report", |h| h.app.snap.as_ref().is_some_and(|s| s.status.is_some()));
        assert!(!h.app.job_active());
    }

    #[test]
    fn it_gives_up_after_the_attempt_limit_and_the_user_can_stop_it() {
        let (mut h, opens) = harness(u32::MAX);
        run_until(&mut h, "giving up", |h| h.app.reconnect.is_none() && h.app.link.is_none() && opens.get() > 0);
        assert_eq!(opens.get(), 8);
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("Could not reconnect")), "{:?}", h.app.status);

        let (mut h, opens) = harness(u32::MAX);
        h.app.reconnect_wait = Duration::from_secs(3600);
        run_until(&mut h, "the loss", |h| h.app.reconnect.is_some());
        h.app.disconnect();
        h.frame(vec![]);
        assert!(h.app.reconnect.is_none());
        assert_eq!(opens.get(), 0, "Disconnect cancels retrying");
    }

    #[test]
    fn the_switch_turns_it_off_and_the_simulator_never_retries() {
        let (mut h, opens) = harness(0);
        h.app.auto_reconnect = false;
        run_until(&mut h, "the loss", |h| h.app.link.is_none() && h.app.status.is_some());
        h.frame(vec![]);
        assert!(h.app.reconnect.is_none() && opens.get() == 0);
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("connection was lost")));

        let (mut h, opens) = harness(0);
        h.app.target = Target::Simulator;
        run_until(&mut h, "the loss", |h| h.app.link.is_none() && h.app.status.is_some());
        assert!(h.app.reconnect.is_none() && opens.get() == 0);
    }
}

mod register_ui_tests {
    use super::*;
    use crate::register_ui::{Marks, Pick};

    #[test]
    fn marks_are_clicked_on_the_canvas_and_applying_them_moves_the_design() {
        let mut h = Harness::new();
        h.app.snapping = crate::snap::SnapSettings { grid: None, objects: false, ..Default::default() };
        h.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 10, "height": 10}));
        for (actual, index, x, y) in [(false, 0, 20.0, 20.0), (false, 1, 120.0, 20.0), (true, 0, 25.0, 22.0), (true, 1, 125.0, 22.0)] {
            h.app.marks.pick = Some(Pick { actual, index });
            h.frame(vec![]);
            h.click(h.at(x, y));
            assert!(h.app.marks.pick.is_none(), "the click sets the mark and ends picking");
        }
        let (design, actual) = h.app.marks.complete().expect("all four marks are set");
        assert!(design[0].distance(Point::new(20.0, 20.0)) < 0.5 && actual[1].distance(Point::new(125.0, 22.0)) < 0.5);
        h.frame(vec![]); // the section and the mark crosses draw
        h.app.run("align.two_point", json!({"design": design.map(|p| [p.x, p.y]), "actual": actual.map(|p| [p.x, p.y]), "lock_scale": true}));
        let b = h.app.session.doc.shapes[0].world_bounds().unwrap();
        assert!((b.x0 - 25.0).abs() < 0.6 && (b.y0 - 22.0).abs() < 0.6, "{b:?}");
    }

    #[test]
    fn escape_cancels_picking_and_incomplete_marks_cannot_be_applied() {
        let mut h = Harness::new();
        h.app.marks.pick = Some(Pick { actual: false, index: 0 });
        h.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        assert!(h.app.marks.pick.is_none());
        assert!(Marks::default().complete().is_none());
        let partial = Marks { design: [Some(Point::new(0.0, 0.0)), None], ..Marks::default() };
        assert!(partial.complete().is_none());
    }
}

mod browser_file_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use laserfoundry_device::Transport;

    use super::*;
    use crate::{PendingAction, Upload};

    #[derive(Default)]
    pub(super) struct Shared {
        /// The file the "person" will choose next.
        next: Option<Upload>,
        requested: Vec<Vec<String>>,
        downloads: Vec<(String, Vec<u8>)>,
        storage: std::collections::HashMap<String, String>,
    }

    struct Browser(Rc<RefCell<Shared>>);

    impl Platform for Browser {
        fn pick_open(&self, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn browser_files(&self) -> bool {
            true
        }

        fn request_upload(&self, extensions: &[&str]) {
            self.0.borrow_mut().requested.push(extensions.iter().map(|e| (*e).to_string()).collect());
        }

        fn take_upload(&self) -> Option<Upload> {
            self.0.borrow_mut().next.take()
        }

        fn download(&self, name: &str, bytes: &[u8], _: &str) {
            self.0.borrow_mut().downloads.push((name.to_string(), bytes.to_vec()));
        }

        fn storage_get(&self, key: &str) -> Option<String> {
            self.0.borrow().storage.get(key).cloned()
        }

        fn storage_set(&self, key: &str, value: &str) {
            self.0.borrow_mut().storage.insert(key.to_string(), value.to_string());
        }

        fn serial_ports(&self) -> Vec<String> {
            Vec::new()
        }

        fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
            Err("none".into())
        }
    }

    pub(super) fn browser() -> (Harness, Rc<RefCell<Shared>>) {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let mut h = Harness {
            ctx: egui::Context::default(),
            app: LaserFoundryApp::new(Session::new(), Box::new(Browser(Rc::clone(&shared)))),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
        };
        h.frame(vec![]);
        (h, shared)
    }

    #[test]
    fn saving_downloads_the_project_and_opening_an_upload_restores_it() {
        let (mut h, shared) = browser();
        h.app.run("shape.rectangle", json!({"x": 5, "y": 6, "width": 20, "height": 10}));
        assert!(crate::menus::save(&mut h.app, false));
        let (name, bytes) = shared.borrow().downloads[0].clone();
        assert_eq!(name, "project.laserfoundry");
        assert!(!h.app.session.dirty, "a downloaded project counts as saved");
        let doc = laserfoundry_format::from_bytes(&bytes).unwrap();
        assert_eq!(doc.shapes.len(), 1);

        h.app.run("file.new", serde_json::Value::Null);
        shared.borrow_mut().next = Some(Upload { name: "x.laserfoundry".into(), bytes });
        h.app.request(PendingAction::Open);
        assert_eq!(shared.borrow().requested.last().unwrap(), &vec!["laserfoundry".to_string()]);
        h.frame(vec![]);
        assert_eq!(h.app.session.doc.shapes.len(), 1);
        assert!(h.app.awaiting_upload.is_none());
    }

    #[test]
    fn recent_projects_are_kept_in_browser_storage_and_reopen_after_a_restart() {
        let (mut h, shared) = browser();
        h.app.run("shape.rectangle", json!({"x": 5, "y": 6, "width": 20, "height": 10}));
        assert!(crate::menus::save(&mut h.app, false));
        let stored = shared.borrow().storage.get(crate::browser_recent::STORAGE_KEY).cloned().unwrap();
        assert!(stored.contains("project.laserfoundry"), "{stored}");

        // A new page load reads the list back, and choosing an entry opens that project.
        let mut again = Harness {
            ctx: egui::Context::default(),
            app: LaserFoundryApp::new(Session::new(), Box::new(Browser(Rc::clone(&shared)))),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
        };
        again.frame(vec![]);
        assert_eq!(again.app.browser_recent.entries.len(), 1);
        assert_eq!(again.app.session.doc.shapes.len(), 0);
        again.app.request(PendingAction::OpenStored("project.laserfoundry".into()));
        assert_eq!(again.app.session.doc.shapes.len(), 1);

        // An uploaded project is remembered too, once; a vanished or damaged entry is dropped from the list.
        let bytes = shared.borrow().downloads[0].1.clone();
        again.app.begin_upload(crate::UploadKind::Project);
        shared.borrow_mut().next = Some(Upload { name: "other.laserfoundry".into(), bytes });
        again.frame(vec![]);
        assert_eq!(
            again.app.browser_recent.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["other.laserfoundry", "project.laserfoundry"]
        );
        again.app.browser_recent.entries[0].data = "!!!".into();
        again.app.request(PendingAction::OpenStored("other.laserfoundry".into()));
        assert_eq!(again.app.browser_recent.entries.len(), 1);
        assert!(!shared.borrow().storage[crate::browser_recent::STORAGE_KEY].contains("other"), "the stored list follows");
        again.app.request(PendingAction::OpenStored("never-existed".into()));
    }

    #[test]
    fn the_host_is_told_when_leaving_would_lose_work() {
        struct Watch(Rc<std::cell::Cell<bool>>);
        impl Platform for Watch {
            fn pick_open(&self, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
                None
            }

            fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
                None
            }

            fn set_leave_warning(&self, on: bool) {
                self.0.set(on);
            }

            fn serial_ports(&self) -> Vec<String> {
                Vec::new()
            }

            fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
                Err("no serial in this test".into())
            }
        }
        let flag = Rc::new(std::cell::Cell::new(true));
        let mut h = Harness {
            ctx: egui::Context::default(),
            app: LaserFoundryApp::new(Session::new(), Box::new(Watch(Rc::clone(&flag)))),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
        };
        h.frame(vec![]);
        assert!(!flag.get(), "an untouched document can be left");
        h.app.run("shape.rectangle", json!({"x": 5, "y": 6, "width": 20, "height": 10}));
        h.frame(vec![]);
        assert!(flag.get(), "unsaved work must be confirmed");
    }

    #[test]
    fn imports_take_uploaded_svg_dxf_and_pictures_and_wait_until_a_file_arrives() {
        let (mut h, shared) = browser();
        h.app.begin_upload(crate::UploadKind::Svg);
        h.frame(vec![]);
        assert!(h.app.awaiting_upload.is_some(), "nothing chosen yet");
        shared.borrow_mut().next = Some(Upload {
            name: "a.svg".into(),
            bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm"><rect width="20" height="10"/></svg>"#.to_vec(),
        });
        h.frame(vec![]);
        assert_eq!(h.app.session.doc.shapes.len(), 1);

        let mut png = Vec::new();
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(4, 4, image::Luma([0])))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        h.app.begin_upload(crate::UploadKind::Image);
        shared.borrow_mut().next = Some(Upload { name: "dot.png".into(), bytes: png });
        h.frame(vec![]);
        assert_eq!(h.app.session.doc.shapes.len(), 2);

        // Junk is reported, not fatal, and does not leave the app waiting.
        h.app.begin_upload(crate::UploadKind::Dxf);
        shared.borrow_mut().next = Some(Upload { name: "bad.dxf".into(), bytes: vec![0xff, 0xfe, 0] });
        h.frame(vec![]);
        assert!(h.app.awaiting_upload.is_none());
        h.app.begin_upload(crate::UploadKind::Project);
        shared.borrow_mut().next = Some(Upload { name: "bad.laserfoundry".into(), bytes: b"{}".to_vec() });
        h.frame(vec![]);
        assert!(matches!(&h.app.status, Some((_, true))), "{:?}", h.app.status);
        assert_eq!(h.app.session.doc.shapes.len(), 2, "a failed open leaves the document alone");
    }

    #[test]
    fn the_browser_loads_keeps_and_clears_a_translation_and_offers_the_template() {
        let (mut h, shared) = browser();
        let catalog = br#"{"name":"Test","font":"/not/readable.ttf","strings":{"Rulers":"Lineale"}}"#.to_vec();
        h.app.choose_translation();
        assert_eq!(shared.borrow().requested.last().unwrap(), &vec!["json".to_string()]);
        shared.borrow_mut().next = Some(Upload { name: "de.json".into(), bytes: catalog.clone() });
        h.frame(vec![]);
        assert_eq!(crate::i18n::tr("Rulers"), "Lineale");
        assert_eq!(crate::i18n::active_font(), None, "the browser cannot read a font path");
        assert!(matches!(&h.app.status, Some((_, false))), "{:?}", h.app.status);
        assert!(shared.borrow().storage["laserfoundry.translation"].contains("Lineale"));

        // The next page load picks it up again.
        crate::i18n::set_active(None);
        let again = LaserFoundryApp::new(Session::new(), Box::new(Browser(Rc::clone(&shared))));
        assert_eq!(crate::i18n::tr("Rulers"), "Lineale");
        drop(again);

        // A damaged catalog is reported and changes nothing; English forgets the stored one.
        h.app.choose_translation();
        shared.borrow_mut().next = Some(Upload { name: "bad.json".into(), bytes: b"[]".to_vec() });
        h.frame(vec![]);
        assert!(matches!(&h.app.status, Some((_, true))), "{:?}", h.app.status);
        assert_eq!(crate::i18n::tr("Rulers"), "Lineale");
        h.app.use_english();
        assert_eq!(crate::i18n::tr("Rulers"), "Rulers");
        assert!(shared.borrow().storage["laserfoundry.translation"].is_empty());

        h.app.save_translation_template();
        let downloads = shared.borrow().downloads.clone();
        assert_eq!(downloads.last().unwrap().0, "translation-template.json");
        assert!(String::from_utf8_lossy(&downloads.last().unwrap().1).contains("\"strings\""));
    }
}

mod font_library_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use laserfoundry_device::Transport;

    use super::*;

    /// A desktop-like host: downloads are allowed and finish when the test says so.
    struct Desktop(Rc<RefCell<Option<Result<FontDownload, FontDownloadFailure>>>>, Rc<RefCell<Vec<bool>>>);

    impl Platform for Desktop {
        fn pick_open(&self, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn font_library_available(&self) -> bool {
            true
        }

        fn start_font_library_download(&self, everything: bool) -> bool {
            self.1.borrow_mut().push(everything);
            self.1.borrow().len() == 1
        }

        fn take_font_library_result(&self) -> Option<Result<FontDownload, FontDownloadFailure>> {
            self.0.borrow_mut().take()
        }

        fn serial_ports(&self) -> Vec<String> {
            Vec::new()
        }

        fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
            Err("none".into())
        }
    }

    fn app_on(platform: Box<dyn Platform>) -> Harness {
        let mut h = Harness {
            ctx: egui::Context::default(),
            app: LaserFoundryApp::new(Session::new(), platform),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
        };
        h.frame(vec![]);
        h
    }

    #[test]
    fn the_text_inspector_lists_only_complete_library_font_folders_by_name() {
        let dir = std::env::temp_dir().join(format!("lf-ui-libfonts-{}", std::process::id()));
        let fonts = dir.join("fonts");
        for (folder, files, name) in [
            ("zeta", &["font.json", "glyphs.stitchfoundry"][..], Some("Alpha Face")),
            ("beta", &["font.json", "glyphs.stitchfoundry"], None),
            ("partial", &["font.json"], Some("No Glyphs")),
        ] {
            std::fs::create_dir_all(fonts.join(folder)).unwrap();
            for f in files {
                let body = match (*f, name) {
                    ("font.json", Some(n)) => format!(r#"{{"name":"{n}"}}"#),
                    _ => "{}".to_string(),
                };
                std::fs::write(fonts.join(folder).join(f), body).unwrap();
            }
        }
        let listed: Vec<String> = crate::panels::library_fonts(&dir).into_iter().map(|(n, _)| n).collect();
        assert_eq!(listed, ["Alpha Face", "beta"], "named by font.json or else the folder, sorted, incomplete ones left out");
        // Ink/Stitch fonts (ltr.svg) are listed too, from their own folder; right-to-left-only ones are not.
        for (folder, file) in [("ink-a", "ltr.svg"), ("ink-rtl", "rtl.svg")] {
            std::fs::create_dir_all(dir.join("inkstitch").join(folder)).unwrap();
            std::fs::write(dir.join("inkstitch").join(folder).join("font.json"), r#"{"name":"Ink Face"}"#).unwrap();
            std::fs::write(dir.join("inkstitch").join(folder).join(file), "<svg/>").unwrap();
        }
        let listed: Vec<String> = crate::panels::library_fonts(&dir).into_iter().map(|(n, _)| n).collect();
        assert_eq!(listed, ["Alpha Face", "Ink Face", "beta"]);
        assert!(crate::panels::library_fonts(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_desktop_host_downloads_in_the_background_and_reports_the_result() {
        let result = Rc::new(RefCell::new(None));
        let started = Rc::new(RefCell::new(Vec::new()));
        let mut h = app_on(Box::new(Desktop(Rc::clone(&result), Rc::clone(&started))));
        h.app.download_font_library(true);
        assert_eq!(*started.borrow(), [true]);
        assert!(h.app.font_download_running);
        h.frame(vec![]);
        assert!(h.app.font_download_running, "still waiting for the result");
        *result.borrow_mut() = Some(Ok(FontDownload::Saved { fetched: 3, kept: 1, folder: "/fonts".into() }));
        h.frame(vec![]);
        assert!(!h.app.font_download_running);
        assert_eq!(
            h.app.status,
            Some((
                "Font library: 3 downloaded, 1 already there, in /fonts. The Foundry fonts are under Library fonts in the text inspector.".into(),
                false
            ))
        );

        // When the host refuses a start (one is already running there), that is said and nothing is waited for.
        h.app.download_font_library(false);
        assert!(!h.app.font_download_running);
        assert!(matches!(&h.app.status, Some((text, false)) if text.contains("already running")), "{:?}", h.app.status);

        h.app.font_download_running = true; // a failed download reports its reason as an error
        *result.borrow_mut() = Some(Err(FontDownloadFailure::Unreachable("timed out".into())));
        h.frame(vec![]);
        assert_eq!(h.app.status, Some(("could not reach the font library: timed out".into(), true)));
    }

    #[test]
    fn font_download_results_read_the_same_in_english() {
        crate::i18n::set_active(None);
        let saved = FontDownload::Saved { fetched: 2, kept: 5, folder: "/data/fonts".into() };
        assert_eq!(
            saved.text(),
            "Font library: 2 downloaded, 5 already there, in /data/fonts. The Foundry fonts are under Library fonts in the text inspector."
        );
        assert_eq!(FontDownload::Loaded { fonts: 4 }.text(), "Font library: 4 fonts ready under Library fonts in the text inspector.");
        for (failure, english) in [
            (FontDownloadFailure::Unreachable("dns".into()), "could not reach the font library: dns"),
            (FontDownloadFailure::BadListing("cut".into()), "the font library listing is not what was expected: cut"),
            (FontDownloadFailure::TooLarge, "the font library is larger than this program will download; refusing"),
            (FontDownloadFailure::Write { path: "a/b".into(), reason: "full".into() }, "could not write a/b: full"),
            (FontDownloadFailure::Cancelled, "the download was cancelled"),
            (FontDownloadFailure::NoFolder, "no folder is available to keep the fonts in"),
            (FontDownloadFailure::Stopped, "the font download stopped unexpectedly"),
            (FontDownloadFailure::SiteUnavailable("404".into()), "The font library is not available from this site (404)"),
            (FontDownloadFailure::NoUsableFonts, "The font library on this site has no usable fonts"),
            (FontDownloadFailure::Other("as found".into()), "as found"),
        ] {
            assert_eq!(failure.text(), english);
        }
    }

    #[test]
    fn a_finished_download_is_worded_in_the_chosen_language() {
        // Regression: the host used to send finished English sentences from its worker thread, where no translation is
        // active, so these messages were never translated. The words are now chosen where the catalog is.
        let catalog = crate::i18n::parse(
            r#"{"strings":{
                "Font library: {fetched} downloaded, {kept} already there, in {folder}. The Foundry fonts are under Library fonts in the text inspector.":"Schriften: {fetched} geladen, {kept} schon da, in {folder}.",
                "could not reach the font library: {detail}":"Schriftenbibliothek nicht erreichbar: {detail}",
                "the download was cancelled":"Der Download wurde abgebrochen"
            }}"#,
        )
        .unwrap();
        let result = Rc::new(RefCell::new(None));
        let mut h = app_on(Box::new(Desktop(Rc::clone(&result), Rc::new(RefCell::new(Vec::new())))));
        crate::i18n::set_active(Some(catalog));

        h.app.font_download_running = true;
        *result.borrow_mut() = Some(Ok(FontDownload::Saved { fetched: 3, kept: 1, folder: "/fonts".into() }));
        h.frame(vec![]);
        assert_eq!(h.app.status, Some(("Schriften: 3 geladen, 1 schon da, in /fonts.".into(), false)));

        h.app.font_download_running = true;
        *result.borrow_mut() = Some(Err(FontDownloadFailure::Unreachable("timed out".into())));
        h.frame(vec![]);
        assert_eq!(h.app.status, Some(("Schriftenbibliothek nicht erreichbar: timed out".into(), true)));

        h.app.font_download_running = true;
        *result.borrow_mut() = Some(Err(FontDownloadFailure::Cancelled));
        h.frame(vec![]);
        assert_eq!(h.app.status, Some(("Der Download wurde abgebrochen".into(), true)));

        crate::i18n::set_active(None);
    }

    #[test]
    fn the_hosted_browser_build_never_offers_or_starts_a_download() {
        // The browser test host does not enable the capability, which is the default for every host.
        let (mut h, _) = super::browser_file_tests::browser();
        assert!(!h.app.platform.font_library_available());
        assert!(h.app.platform.font_library_dir().is_none());
        h.app.download_font_library(true);
        h.app.download_font_library(false);
        assert!(!h.app.font_download_running);
        assert_eq!(h.app.status, None, "nothing is said, nothing is started");
    }
}

mod async_serial_tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use laserfoundry_device::{JobState, SimulatedGrbl, Transport};

    use super::*;

    type Ready = Rc<RefCell<Option<Result<Box<dyn Transport + Send>, String>>>>;

    /// A host like the browser: the device is chosen asynchronously, and there are no threads.
    struct AsyncHost {
        ready: Ready,
        requests: Rc<RefCell<u32>>,
    }

    impl Platform for AsyncHost {
        fn pick_open(&self, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn inline_link(&self) -> bool {
            true
        }

        fn serial_async(&self) -> bool {
            true
        }

        fn request_serial(&self) {
            *self.requests.borrow_mut() += 1;
        }

        fn take_serial(&self) -> Option<Result<Box<dyn Transport + Send>, String>> {
            self.ready.borrow_mut().take()
        }

        fn serial_ports(&self) -> Vec<String> {
            vec!["USB device".into()]
        }

        fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
            Err("use request_serial".into())
        }
    }

    fn host() -> (Harness, Ready, Rc<RefCell<u32>>) {
        let ready: Ready = Rc::new(RefCell::new(None));
        let requests = Rc::new(RefCell::new(0));
        let mut h = Harness {
            ctx: egui::Context::default(),
            app: LaserFoundryApp::new(Session::new(), Box::new(AsyncHost { ready: Rc::clone(&ready), requests: Rc::clone(&requests) })),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
        };
        h.frame(vec![]);
        (h, ready, requests)
    }

    fn pump(h: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
        let start = Instant::now();
        while !done(h) {
            h.frame(vec![]);
            assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}: {:?}", h.app.status);
        }
    }

    #[test]
    fn connecting_asks_the_host_to_pick_a_device_and_the_link_runs_on_frames() {
        let (mut h, ready, requests) = host();
        h.app.target = Target::Serial("USB device".into());
        h.app.connect();
        assert_eq!(*requests.borrow(), 1);
        h.frame(vec![]);
        assert!(h.app.link.is_none() && h.app.awaiting_serial, "still waiting for the person to choose");

        let mut sim = SimulatedGrbl::new();
        sim.set_non_blocking(true);
        *ready.borrow_mut() = Some(Ok(Box::new(sim)));
        pump(&mut h, "the link", |h| h.app.link.is_some() && h.app.snap.as_ref().is_some_and(|s| s.status.is_some()));
        assert!(!h.app.awaiting_serial);

        h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 10}));
        let program = h.app.run("job.gcode", json!({"include": true})).and_then(|v| v["gcode"].as_str().map(String::from)).unwrap();
        h.app.start_job(program);
        pump(&mut h, "the job", |h| matches!(h.app.snap.as_ref().map(|s| &s.job), Some(JobState::Finished(_))));
        assert!(matches!(h.app.snap.as_ref().map(|s| &s.job), Some(JobState::Finished(Ok(_)))), "{:?}", h.app.status);
    }

    #[test]
    fn a_refused_or_cancelled_choice_is_reported_and_the_simulator_works_without_threads() {
        let (mut h, ready, _) = host();
        h.app.target = Target::Serial("USB device".into());
        h.app.connect();
        *ready.borrow_mut() = Some(Err("No device selected".into()));
        h.frame(vec![]);
        assert!(h.app.link.is_none() && !h.app.awaiting_serial);
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("No device selected")), "{:?}", h.app.status);

        h.app.target = Target::Simulator;
        h.app.connect();
        pump(&mut h, "the simulated machine", |h| h.app.snap.as_ref().is_some_and(|s| s.status.is_some()));
        assert!(h.app.reconnect.is_none(), "no automatic reconnect for hosts that need a click to open a device");
    }
}

mod autosave_generation_tests {
    use super::*;
    use crate::recovery::{GENERATIONS, generation_path, newest_readable, rotate};

    #[test]
    fn rotation_keeps_a_few_copies_newest_first_and_drops_the_oldest() {
        let dir = temp_dir("generations");
        let base = dir.join("recovery.laserfoundry");
        for i in 0..5 {
            rotate(&base);
            std::fs::write(&base, format!("copy {i}")).unwrap();
        }
        let read = |n| std::fs::read_to_string(generation_path(&base, n)).unwrap();
        assert_eq!((read(0), read(1), read(2)), ("copy 4".to_string(), "copy 3".to_string(), "copy 2".to_string()));
        assert!(!generation_path(&base, GENERATIONS).exists(), "no copy beyond the limit");
        assert_eq!(generation_path(&base, 1).file_name().unwrap(), "recovery.1.laserfoundry");
        rotate(&dir.join("nothing-here.laserfoundry")); // nothing to rotate is fine
    }

    #[test]
    fn a_damaged_newest_copy_falls_back_to_an_older_readable_one_and_all_copies_are_cleared_when_saved() {
        let dir = temp_dir("fallback");
        let base = dir.join("recovery.laserfoundry");
        let mut good = laserfoundry_doc::Document::default();
        good.add_shape(laserfoundry_doc::Shape::path("r", 0, laserfoundry_geom::rect_path(0.0, 0.0, 5.0, 5.0)));
        laserfoundry_format::save(&generation_path(&base, 1), &good).unwrap();
        std::fs::write(&base, b"{ truncated").unwrap();
        assert_eq!(newest_readable(&base), Some(generation_path(&base, 1)));

        let mut h = harness_with(RecoveryPlatform { file: base.clone(), open_answer: None, recent: None });
        assert_eq!(h.app.pending_recovery, Some(generation_path(&base, 1)), "the readable older copy is offered");
        h.app.answer_recovery(true);
        assert_eq!(h.app.session.doc.shapes.len(), 1);
        assert!(h.app.session.dirty);
        h.app.run("file.save", json!({"path": dir.join("saved.laserfoundry")}));
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(!base.exists() && !generation_path(&base, 1).exists(), "saved work leaves no stale backups");
    }

    #[test]
    fn autosaves_rotate_in_the_running_app_and_every_write_is_checked() {
        let dir = temp_dir("running");
        let base = dir.join("recovery.laserfoundry");
        let mut h = harness_with(RecoveryPlatform { file: base.clone(), open_answer: None, recent: None });
        h.app.autosave_interval = Duration::ZERO;
        for i in 0..3 {
            h.app.run("shape.rectangle", json!({"x": i * 10, "width": 5, "height": 5}));
            h.frame(vec![]);
        }
        assert!(base.exists() && generation_path(&base, 1).exists() && generation_path(&base, 2).exists());
        let counts: Vec<usize> = (0..3).map(|n| laserfoundry_format::load(&generation_path(&base, n)).unwrap().shapes.len()).collect();
        assert_eq!(counts, vec![3, 2, 1]);
        assert!(!matches!(&h.app.status, Some((m, true)) if m.contains("backup")));
    }
}

mod preset_library_tests {
    use super::*;

    #[test]
    fn the_library_merges_both_ways_and_survives_damaged_files() {
        let dir = temp_dir("library");
        let file = dir.join("deep").join("presets.json");
        let mut h = Harness::new();
        h.app.run("layer.set", json!({"id": 0, "speed": 250}));
        h.app.run("preset.save", json!({"name": "A", "layer": 0}));
        h.app.save_preset_library(&file);
        h.app.run("preset.save", json!({"name": "B", "layer": 0}));
        h.app.save_preset_library(&file);
        let stored: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(stored.len(), 2, "saving again only adds what is new");

        let mut other = Harness::new();
        other.app.load_preset_library(&file);
        assert_eq!(other.app.session.doc.presets.len(), 2);
        assert_eq!(other.app.session.doc.presets[0].settings.speed, 250.0);

        for bad in [&b"not json"[..], b"{\"a\":1}", b"[1,2,3]", b"[{\"name\":\"x\"}]"] {
            std::fs::write(&file, bad).unwrap();
            let mut h = Harness::new();
            h.app.load_preset_library(&file); // reports, never panics
            assert!(h.app.session.doc.presets.is_empty());
        }
        let mut h = Harness::new();
        h.app.load_preset_library(&dir.join("missing.json"));
        assert!(matches!(&h.app.status, Some((m, false)) if m.contains("empty")), "{:?}", h.app.status);
    }
}

mod template_tests {
    use laserfoundry_device::Transport;

    use super::*;
    use crate::PendingAction;

    struct Templates(std::path::PathBuf);

    impl Platform for Templates {
        fn pick_open(&self, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn templates_dir(&self) -> Option<std::path::PathBuf> {
            Some(self.0.clone())
        }

        fn serial_ports(&self) -> Vec<String> {
            Vec::new()
        }

        fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
            Err("none".into())
        }
    }

    fn harness(dir: &std::path::Path) -> Harness {
        let mut h = Harness {
            ctx: egui::Context::default(),
            app: LaserFoundryApp::new(Session::new(), Box::new(Templates(dir.to_path_buf()))),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
        };
        h.frame(vec![]);
        h
    }

    #[test]
    fn a_template_is_saved_listed_and_opens_as_a_new_untitled_project() {
        let dir = temp_dir("templates").join("t");
        let mut h = harness(&dir);
        assert!(h.app.templates().is_empty(), "a missing folder is an empty list");
        h.app.run("machine.set", json!({"bed_width": 123}));
        h.app.run("layer.set", json!({"id": 0, "speed": 777}));
        h.app.save_template("  My ../setup! ");
        let list = h.app.templates();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "My setup", "only plain characters survive in the name");
        assert!(list[0].1.starts_with(&dir), "the file stays inside the templates folder");
        assert!(h.app.session.dirty, "saving a template does not count as saving the project");

        let mut other = harness(&dir);
        other.app.request(PendingAction::FromTemplate(list[0].1.clone()));
        assert_eq!(other.app.session.doc.machine.bed_width, 123.0);
        assert_eq!(other.app.session.doc.layer(0).unwrap().speed, 777.0);
        assert!(other.app.session.path.is_none() && other.app.session.dirty, "untitled, unsaved");
        assert!(list[0].1.exists());
        h.app.save_template("   ");
        assert!(matches!(&h.app.status, Some((m, true)) if m.contains("name")));
        for i in 0..3 {
            h.app.save_template(&format!("t{i}"));
        }
        assert_eq!(h.app.templates().len(), 4);
    }
}

mod theme_tests {
    use super::*;
    use crate::theme::{ThemeChoice, Tokens};

    #[test]
    fn the_theme_choice_switches_egui_and_the_canvas_colours() {
        let mut h = Harness::new();
        for (choice, dark) in [(ThemeChoice::Light, false), (ThemeChoice::Dark, true), (ThemeChoice::Light, false)] {
            h.app.theme = choice;
            for _ in 0..3 {
                h.frame(vec![]);
            }
            let expected = if dark { Tokens::DARK.workspace } else { Tokens::LIGHT.workspace };
            assert_eq!(h.app.tokens.workspace, expected, "{choice:?}");
            assert_eq!(h.ctx.global_style().visuals.dark_mode, dark);
        }
        assert_eq!(ThemeChoice::ALL.len(), 3);
        h.app.theme = ThemeChoice::Auto;
        h.frame(vec![]); // follows the system without panicking
    }

    #[test]
    fn the_light_and_dark_palettes_are_different_and_every_token_is_visible_on_its_background() {
        let luma = |c: egui::Color32| 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
        assert!(luma(Tokens::LIGHT.workspace) > luma(Tokens::DARK.workspace));
        for t in [&Tokens::LIGHT, &Tokens::DARK] {
            for fg in [t.selection, t.error, t.ok, t.origin, t.ruler_text] {
                assert!((luma(fg) - luma(t.bed)).abs() > 40.0 || (luma(fg) - luma(t.ruler)).abs() > 40.0, "{fg:?}");
            }
        }
    }
}
