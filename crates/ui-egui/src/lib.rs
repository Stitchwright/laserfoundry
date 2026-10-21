//! The egui front end. Thin by design: panels read engine state and act only through
//! `Session::execute`, so everything the UI can do an agent or script can do too.

mod canvas;
mod crop_ui;
mod font_ui;
mod geometry;
mod head_ui;
mod history_ui;
/// Translation catalogs live in their own crate so the command line can use them too.
pub(crate) mod i18n {
    pub use laserfoundry_i18n::*;
}
mod ai_ui;
mod browser_recent;
mod job_position;
mod machine_report;
mod menus;
mod nodes_ui;
mod panels;
mod platform;
mod recent;
mod recovery;
mod register_ui;
mod select_size_ui;
mod settings_ui;
mod shortcuts;
pub(crate) mod snap;
mod spacing;
mod tablet;
mod theme;
mod timeline_ui;
mod view;

pub use platform::{FontDownload, FontDownloadFailure, NoPlatform, Platform, Upload};

use crate::i18n::{tr, trf};
use std::path::PathBuf;
use std::time::Duration;

use web_time::Instant;

use laserfoundry_device::{JobState, Link, SimulatedGrbl, Snapshot};
use laserfoundry_doc::LayerId;
use laserfoundry_engine::{EngineError, Session};
use serde_json::Value;

use crate::geometry::{GeometryCache, PreviewCache};
use crate::theme::Tokens;
use crate::view::View;

/// Interactive tools on the left toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Line,
    Polygon,
    Text,
    /// Edit the nodes and handles of the selected path.
    Nodes,
    /// Draw Bézier paths node by node.
    Pen,
    /// Drag the view (for fingers, which have no middle button).
    Hand,
}

/// Where Start sends the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Simulator,
    Serial(String),
}

/// Attempts made to get a lost serial connection back before giving up.
const RECONNECT_ATTEMPTS: u32 = 8;
/// Longest wait between attempts.
const RECONNECT_MAX_WAIT: Duration = Duration::from_secs(30);

/// A lost connection being retried.
struct Reconnect {
    attempts: u32,
    next: Instant,
}

pub struct LaserFoundryApp {
    pub session: Session,
    platform: Box<dyn Platform>,
    tokens: Tokens,
    theme: theme::ThemeChoice,
    /// The theme last handed to egui, so it is only changed when the choice changes.
    applied_theme: Option<theme::ThemeChoice>,
    /// Tablet mode: the setting, whether a touch has been seen, what was last applied, and its helpers.
    tablet: tablet::TabletMode,
    touch_seen: bool,
    applied_tablet: Option<bool>,
    /// A tap-able stand-in for holding Shift.
    sticky_shift: bool,
    show_inspector: bool,
    inspector_decided: bool,
    /// Where the canvas context menu was opened (right-click, or a long press on a touch screen).
    menu_point: Option<laserfoundry_geom::Point>,
    /// Frames left in which the view is refitted after the interface scale changed.
    refit_frames: u8,
    /// Where the tablet action bar drew its buttons (the frame tests click them).
    #[cfg(test)]
    button_rects: std::collections::HashMap<&'static str, egui::Rect>,
    view: Option<View>,
    tool: Tool,
    drag: canvas::Drag,
    active_layer: LayerId,
    show_preview: bool,
    show_travel: bool,
    preview_colors: timeline_ui::PreviewColors,
    polygon_sides: u32,
    status: Option<(String, bool)>,
    geometry: GeometryCache,
    thumbs: crate::geometry::ImageTextures,
    preview: PreviewCache,
    player: timeline_ui::Player,
    link: Option<Link>,
    /// Latest state of the machine link, refreshed once per frame.
    snap: Option<Snapshot>,
    last_job: Option<JobState>,
    console_input: String,
    /// The text being typed in the inspector (committed when the box loses focus).
    text_buffer: Option<(laserfoundry_doc::ShapeId, String)>,
    text_warning: Option<String>,
    jog_step: f64,
    offset_distance: f64,
    corner_radius: f64,
    array_columns: u32,
    array_rows: u32,
    array_gap: f64,
    array_count: u32,
    nest_gap: f64,
    nest_turn: bool,
    snapping: snap::SnapSettings,
    show_rulers: bool,
    guides: Vec<canvas::Guide>,
    node_sel: Option<nodes_ui::NodeSel>,
    node_drag: Option<nodes_ui::NodeDrag>,
    pen: Option<nodes_ui::Pen>,
    /// Cursor position on the canvas in mm (hover, not only while pressed).
    hover_world: Option<laserfoundry_geom::Point>,
    /// Where the canvas was drawn last frame (screen points).
    canvas_rect: Option<egui::Rect>,
    /// Alignment lines the last drag locked onto, for drawing: (x, y).
    snap_lines: canvas::SnapLines,
    /// Equal gaps the last move locked onto, for drawing.
    snap_spacing: Vec<spacing::Gap>,
    /// Points acquired by hovering with a drawing tool; horizontal and vertical lines through them attract the cursor.
    tracked: Vec<laserfoundry_geom::Point>,
    /// The point being hovered with the time it was first hovered, until it is acquired.
    track_candidate: Option<(laserfoundry_geom::Point, f64)>,
    grid_cols: u32,
    grid_rows: u32,
    grid_speed: (f64, f64),
    grid_power: (f64, f64),
    grid_size: f64,
    array_radius: f64,
    offset_join: &'static str,
    offset_keep: bool,
    jog_feed: f64,
    target: Target,
    ports: Vec<String>,
    frame_power: f64,
    autosave_interval: Duration,
    autosave_revision: Option<u64>,
    last_autosave: Instant,
    /// A recovery file found at startup that the person has not yet decided about.
    pending_recovery: Option<PathBuf>,
    /// An action waiting for "save changes?" to be answered.
    pending_action: Option<PendingAction>,
    recent: recent::Recent,
    /// Recent projects in a browser, which keeps their contents instead of their paths.
    browser_recent: browser_recent::BrowserRecent,
    shortcuts: shortcuts::Shortcuts,
    shortcuts_open: bool,
    history_open: bool,
    select_size_open: bool,
    fonts_open: bool,
    font_manager: font_ui::FontManager,
    size_filter: select_size_ui::SizeFilter,
    /// The fonts must be rebuilt (a translation with its own font was loaded or removed).
    fonts_dirty: bool,
    /// The action waiting for its new key combination.
    capturing: Option<shortcuts::Action>,
    shortcut_error: Option<String>,
    /// Percentages to cut off the left, top, right and bottom of the selected image.
    crop_pct: [f64; 4],
    /// Dragging the edges of the selected picture on the canvas shows and sets `crop_pct`.
    crop_mode: bool,
    crop_drag: Option<usize>,
    /// The picture the crop percentages belong to.
    crop_target: Option<u64>,
    sharpen_amount: f64,
    resample_scale: f64,
    trace_threshold: f64,
    trace_smooth: bool,
    preset_name: String,
    template_name: String,
    resume_line: usize,
    /// The program of the job being run, kept to work out where to restart it if it is interrupted.
    job_program: Option<std::sync::Arc<str>>,
    last_position_write: Instant,
    /// A job a previous session left unfinished, found at startup.
    interrupted: Option<job_position::JobPosition>,
    /// What a requested upload is for.
    awaiting_upload: Option<UploadKind>,
    /// A program waiting for the person to confirm a start the bed cannot be checked for (relative start).
    pending_start: Option<String>,
    ai: ai_ui::AiState,
    /// A web address to open in the browser at the next frame (a payment page).
    pending_url: Option<String>,
    /// The controller's settings were asked for on this connection (so jobs can be refused when laser mode is off).
    settings_asked: bool,
    /// The person chose not to be asked again about relative starts in this session.
    relative_start_trusted: bool,
    /// Waiting for the host to finish choosing and opening a serial device.
    awaiting_serial: bool,
    /// A font library download is running in the background.
    font_download_running: bool,
    /// Backups from an earlier session were recovered and still sit on disk until the work is saved.
    stale_backups: bool,
    marks: register_ui::Marks,
    auto_reconnect: bool,
    reconnect: Option<Reconnect>,
    /// First wait before retrying; doubles each attempt (tests shrink it).
    reconnect_wait: Duration,
    /// While on, a click on the canvas sends the laser head there.
    move_head_mode: bool,
    /// x, y, width, height typed into the "create by numbers" fields.
    new_shape: [f64; 4],
    /// The project path last added to the recent list.
    recorded_path: Option<PathBuf>,
    allow_close: bool,
    last_title: String,
}

/// Where a browser-style host keeps the chosen translation catalog.
const TRANSLATION_KEY: &str = "laserfoundry.translation";

/// What a file chosen in a browser-style host is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadKind {
    Project,
    Svg,
    Dxf,
    Image,
    Translation,
    /// A picture for the AI features (see `ai_ui`).
    AiPicture,
}

impl UploadKind {
    fn extensions(self) -> &'static [&'static str] {
        match self {
            UploadKind::Project => &["laserfoundry"],
            UploadKind::Svg => &["svg"],
            UploadKind::Dxf => &["dxf"],
            UploadKind::Image => &["png", "jpg", "jpeg", "bmp", "gif", "webp"],
            UploadKind::Translation => &["json"],
            UploadKind::AiPicture => &["png", "jpg", "jpeg", "webp", "gif", "bmp"],
        }
    }
}

/// Actions that would discard unsaved changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingAction {
    New,
    Open,
    OpenPath(PathBuf),
    /// A project remembered by a browser host, by name.
    OpenStored(String),
    /// A template: opens as a new untitled project.
    FromTemplate(PathBuf),
    Close,
}

/// How often a changed document is written to the recovery file.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(20);

impl LaserFoundryApp {
    pub fn new(session: Session, platform: Box<dyn Platform>) -> Self {
        let ports = platform.serial_ports();
        // Offer the newest autosave that still reads back; a damaged newest copy falls back to an older one.
        let pending_recovery = platform.recovery_file().and_then(|base| recovery::newest_readable(&base).or_else(|| base.exists().then_some(base)));
        let interrupted = platform.job_position_file().and_then(|p| job_position::load(&p));
        let recent = platform.recent_file().map(|p| recent::Recent::load(&p)).unwrap_or_default();
        let browser_recent = if platform.browser_files() {
            platform.storage_get(browser_recent::STORAGE_KEY).map(|t| browser_recent::BrowserRecent::parse(&t)).unwrap_or_default()
        } else {
            browser_recent::BrowserRecent::default()
        };
        let shortcuts = platform.shortcuts_file().map(|p| shortcuts::Shortcuts::load(&p)).unwrap_or_default();
        let mut app = Self {
            session,
            platform,
            tokens: Tokens::DARK,
            theme: theme::ThemeChoice::Auto,
            applied_theme: None,
            tablet: tablet::TabletMode::default(),
            touch_seen: false,
            applied_tablet: None,
            sticky_shift: false,
            show_inspector: true,
            inspector_decided: false,
            menu_point: None,
            refit_frames: 0,
            #[cfg(test)]
            button_rects: std::collections::HashMap::new(),
            view: None,
            tool: Tool::Select,
            drag: canvas::Drag::None,
            active_layer: 0,
            show_preview: false,
            show_travel: false,
            preview_colors: timeline_ui::PreviewColors::Layer,
            polygon_sides: 6,
            status: None,
            geometry: GeometryCache::default(),
            thumbs: Default::default(),
            preview: PreviewCache::default(),
            player: timeline_ui::Player::default(),
            link: None,
            snap: None,
            last_job: None,
            console_input: String::new(),
            text_buffer: None,
            text_warning: None,
            jog_step: 1.0,
            offset_distance: 1.0,
            corner_radius: 0.0,
            array_columns: 3,
            array_rows: 3,
            array_gap: 5.0,
            array_count: 6,
            nest_gap: 2.0,
            nest_turn: true,
            snapping: snap::SnapSettings::default(),
            show_rulers: true,
            guides: Vec::new(),
            node_sel: None,
            node_drag: None,
            pen: None,
            hover_world: None,
            canvas_rect: None,
            snap_lines: (None, None, None),
            snap_spacing: Vec::new(),
            tracked: Vec::new(),
            track_candidate: None,
            grid_cols: 5,
            grid_rows: 5,
            grid_speed: (200.0, 1200.0),
            grid_power: (10.0, 60.0),
            grid_size: 10.0,
            array_radius: 40.0,
            offset_join: "round",
            offset_keep: true,
            jog_feed: 3000.0,
            target: Target::Simulator,
            ports,
            frame_power: 0.0,
            autosave_interval: AUTOSAVE_INTERVAL,
            autosave_revision: None,
            last_autosave: Instant::now(),
            pending_recovery,
            pending_action: None,
            recent,
            browser_recent,
            shortcuts,
            shortcuts_open: false,
            history_open: false,
            select_size_open: false,
            fonts_open: false,
            font_manager: font_ui::FontManager::default(),
            size_filter: select_size_ui::SizeFilter::default(),
            fonts_dirty: false,
            capturing: None,
            shortcut_error: None,
            crop_pct: [0.0; 4],
            crop_mode: false,
            crop_drag: None,
            crop_target: None,
            sharpen_amount: 1.0,
            resample_scale: 0.5,
            trace_threshold: 128.0,
            trace_smooth: true,
            preset_name: String::new(),
            template_name: String::new(),
            resume_line: 1,
            job_program: None,
            last_position_write: Instant::now(),
            interrupted,
            awaiting_upload: None,
            awaiting_serial: false,
            font_download_running: false,
            stale_backups: false,
            marks: register_ui::Marks::default(),
            auto_reconnect: true,
            reconnect: None,
            reconnect_wait: Duration::from_secs(2),
            move_head_mode: false,
            new_shape: [10.0, 10.0, 50.0, 30.0],
            recorded_path: None,
            allow_close: false,
            pending_start: None,
            ai: ai_ui::AiState::default(),
            pending_url: None,
            settings_asked: false,
            relative_start_trusted: false,
            last_title: String::new(),
        };
        app.load_saved_translation();
        app.load_ai();
        app
    }

    /// Draws one frame. Called from the host's `eframe::App::ui`.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.apply_theme(ui.ctx());
        tablet::begin_frame(self, ui.ctx());
        self.apply_translation_font(ui.ctx());
        self.poll_link(ui.ctx());
        self.autosave_tick(ui.ctx());
        self.watch_close_and_title(ui.ctx());
        self.platform.set_leave_warning(self.job_active() || self.session.dirty);
        self.poll_ai(ui.ctx());
        if let Some(url) = self.pending_url.take() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
        self.track_recent();
        self.poll_upload(ui.ctx());
        self.poll_font_download(ui.ctx());
        self.forget_crop_for_other_pictures();
        self.poll_serial(ui.ctx());
        if !self.has_modal() {
            self.handle_shortcuts(ui.ctx());
        }
        menus::menu_bar(self, ui);
        if self.tablet_active() {
            tablet::action_bar(self, ui);
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        let tools_width = if self.tablet_active() { 112.0 } else { 84.0 };
        egui::Panel::left("tools").resizable(false).exact_size(tools_width).show(ui, |ui| panels::toolbar(self, ui));
        if self.show_inspector {
            egui::Panel::right("inspector").default_size(300.0).show(ui, |ui| panels::inspector(self, ui));
        }
        if self.show_preview {
            egui::Panel::bottom("timeline").show(ui, |ui| timeline_ui::bar(self, ui));
        }
        egui::CentralPanel::default().show(ui, |ui| canvas::show(self, ui));
        self.dialogs(ui.ctx());
        self.ai_windows(ui.ctx());
    }

    /// Loads the saved translation, if there is one. A damaged file is reported and ignored.
    fn load_saved_translation(&mut self) {
        if self.platform.browser_files() {
            // The browser keeps the catalog's text in local storage.
            if let Some(text) = self.platform.storage_get(TRANSLATION_KEY).filter(|t| !t.is_empty()) {
                match i18n::parse(&text) {
                    Ok(t) => i18n::set_active(Some(t)),
                    Err(e) => self.status = Some((trf!("Could not use the saved translation: {e}", e = e), true)),
                }
            }
            return;
        }
        let Some(path) = self.platform.translation_file().filter(|p| p.exists()) else { return };
        match i18n::load(&path) {
            Ok(t) => {
                i18n::set_active(Some(t));
                self.fonts_dirty = true;
            }
            Err(e) => self.status = Some((trf!("Could not use the saved translation: {e}", e = e), true)),
        }
    }

    /// Adds the translation's font (if it names one) behind egui's own, or restores the default fonts.
    fn apply_translation_font(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.fonts_dirty) {
            return;
        }
        let mut fonts = egui::FontDefinitions::default();
        if let Some(path) = i18n::active_font() {
            match i18n::read_font(&path) {
                Ok(bytes) => {
                    fonts.font_data.insert("translation".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                        fonts.families.entry(family).or_default().push("translation".into());
                    }
                }
                Err(e) => self.status = Some((e, true)),
            }
        }
        ctx.set_fonts(fonts);
    }

    /// Asks for a translation file, keeps a copy as the saved translation, and starts using it.
    pub(crate) fn choose_translation(&mut self) {
        if self.platform.browser_files() {
            self.begin_upload(UploadKind::Translation);
            return;
        }
        let Some(path) = self.pick_open("Choose a translation", &["json"]) else { return };
        match i18n::load(&path) {
            Ok(t) => {
                if let Some(saved) = self.platform.translation_file() {
                    if let Some(parent) = saved.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = std::fs::copy(&path, &saved) {
                        log::warn!("could not keep a copy of the translation at {}: {e}", saved.display());
                    }
                }
                self.status =
                    Some((trf!("Using the translation {name}", name = if t.name.is_empty() { tr("(unnamed)") } else { t.name.clone() }), false));
                i18n::set_active(Some(t));
                self.fonts_dirty = true;
            }
            Err(e) => self.status = Some((e, true)),
        }
    }

    /// Uses a catalog chosen in a browser-style host and keeps its text for the next visit. The catalog's
    /// `font` names a file the browser cannot read, so it is ignored there.
    fn use_uploaded_translation(&mut self, bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(bytes);
        match i18n::parse(&text) {
            Ok(mut t) => {
                t.font = None;
                self.platform.storage_set(TRANSLATION_KEY, &text);
                self.status =
                    Some((trf!("Using the translation {name}", name = if t.name.is_empty() { tr("(unnamed)") } else { t.name.clone() }), false));
                i18n::set_active(Some(t));
                self.fonts_dirty = true;
            }
            Err(e) => self.status = Some((e, true)),
        }
        false
    }

    /// Goes back to English and forgets the saved translation.
    pub(crate) fn use_english(&mut self) {
        i18n::set_active(None);
        self.fonts_dirty = true;
        if self.platform.browser_files() {
            self.platform.storage_set(TRANSLATION_KEY, "");
            return;
        }
        if let Some(saved) = self.platform.translation_file()
            && let Err(e) = std::fs::remove_file(&saved)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not remove {}: {e}", saved.display());
        }
    }

    /// Writes every string the interface has shown so far as a catalog to translate.
    pub(crate) fn save_translation_template(&mut self) {
        if self.platform.browser_files() {
            self.download("translation-template.json", i18n::template().as_bytes(), "application/json");
            return;
        }
        let Some(path) = self.pick_save("Save translation template", "translation-template.json", &["json"]) else { return };
        self.status = Some(match std::fs::write(&path, i18n::template()) {
            Ok(()) => (trf!("Saved the translation template to {path}", path = path.display()), false),
            Err(e) => (trf!("Could not save the template: {e}", e = e), true),
        });
    }

    /// Hands the chosen theme to egui and picks matching canvas colours.
    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme != Some(self.theme) {
            ctx.set_theme(self.theme.preference());
            self.applied_theme = Some(self.theme);
        }
        self.tokens = if ctx.global_style().visuals.dark_mode { Tokens::DARK } else { Tokens::LIGHT };
    }

    fn has_modal(&self) -> bool {
        self.pending_recovery.is_some() || self.pending_action.is_some() || self.pending_start.is_some() || self.capturing.is_some()
    }

    /// Writes the recovery copy while there are unsaved changes, and removes it once they are saved.
    fn autosave_tick(&mut self, ctx: &egui::Context) {
        let Some(path) = self.platform.recovery_file() else { return };
        // Until the person has decided about the previous session's copy, do not overwrite it.
        if self.pending_recovery.is_some() {
            return;
        }
        if !self.session.dirty {
            if std::mem::take(&mut self.stale_backups) | self.autosave_revision.take().is_some() {
                self.clear_recovery();
            }
            return;
        }
        if self.autosave_revision == Some(self.session.revision) {
            return;
        }
        // egui only repaints on input; without this an idle window would never get to autosave.
        let wait = self.autosave_interval.saturating_sub(self.last_autosave.elapsed());
        if !wait.is_zero() {
            ctx.request_repaint_after(wait);
            return;
        }
        self.last_autosave = Instant::now();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent); // a missing folder surfaces as the save error below
        }
        recovery::rotate(&path);
        match self.session.execute("file.autosave", &serde_json::json!({ "path": path })) {
            Ok(_) => {
                self.autosave_revision = Some(self.session.revision);
                // A copy that cannot be read back is no safety net: say so while it can still be fixed.
                if !recovery::is_readable(&path) {
                    self.status = Some((tr("The automatic backup could not be read back; save your work"), true));
                }
            }
            Err(e) => log::warn!("autosave failed: {e}"),
        }
    }

    /// Removes the autosave and its older copies (the work was saved or knowingly discarded).
    fn clear_recovery(&mut self) {
        self.platform.clear_recovery();
        if let Some(base) = self.platform.recovery_file() {
            recovery::remove_all(&base);
        }
    }

    /// Keeps the window title current and turns a close request on unsaved work into a question.
    fn watch_close_and_title(&mut self, ctx: &egui::Context) {
        let title = window_title(&self.session);
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.session.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_action = Some(PendingAction::Close);
        }
    }

    /// Crop percentages belong to one picture: selecting another (or nothing) starts them afresh.
    fn forget_crop_for_other_pictures(&mut self) {
        let current = match self.session.selection.as_slice() {
            [id] if crop_ui::selected_image(self).is_some() => Some(*id),
            _ => None,
        };
        if current != self.crop_target {
            self.crop_target = current;
            self.crop_pct = [0.0; 4];
            self.crop_drag = None;
            if current.is_none() {
                self.crop_mode = false;
            }
        }
    }

    /// Asks a browser-style host for a file; [`poll_upload`](Self::poll_upload) applies it.
    pub(crate) fn begin_upload(&mut self, kind: UploadKind) {
        self.awaiting_upload = Some(kind);
        self.platform.request_upload(kind.extensions());
    }

    /// Starts the font library download if this host allows it.
    pub(crate) fn download_font_library(&mut self, everything: bool) {
        if !self.platform.font_library_available() {
            return;
        }
        if self.platform.start_font_library_download(everything) {
            self.font_download_running = true;
            self.say(tr("Downloading the font library…"));
        } else {
            self.say(tr("A font download is already running"));
        }
    }

    fn poll_font_download(&mut self, ctx: &egui::Context) {
        if !self.font_download_running {
            return;
        }
        match self.platform.take_font_library_result() {
            Some(result) => {
                self.font_download_running = false;
                match result {
                    Ok(done) => self.say(done.text()),
                    Err(failure) => self.status = Some((failure.text(), true)),
                }
            }
            None => ctx.request_repaint_after(Duration::from_millis(300)),
        }
    }

    fn poll_upload(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.awaiting_upload else { return };
        let Some(upload) = self.platform.take_upload() else {
            ctx.request_repaint_after(Duration::from_millis(200));
            return;
        };
        self.awaiting_upload = None;
        let data = laserfoundry_doc::base64::encode(&upload.bytes);
        let name = upload.name;
        let ok = match kind {
            UploadKind::Project => {
                let ok = self.run("file.open", serde_json::json!({ "data_base64": data })).is_some();
                if ok {
                    self.clear_recovery();
                    self.autosave_revision = None;
                    self.view = None;
                    self.remember_project(&name, &upload.bytes);
                }
                ok
            }
            UploadKind::Svg => self.run("file.import_svg", serde_json::json!({ "data": String::from_utf8_lossy(&upload.bytes) })).is_some(),
            UploadKind::Dxf => self.run("file.import_dxf", serde_json::json!({ "data": String::from_utf8_lossy(&upload.bytes) })).is_some(),
            UploadKind::Image => self.run("file.import_image", serde_json::json!({ "data_base64": data, "name": name })).is_some(),
            UploadKind::Translation => self.use_uploaded_translation(&upload.bytes),
            UploadKind::AiPicture => self.ai_picture_arrived(&name, &data),
        };
        if ok {
            self.say(trf!("Opened {name}", name = name));
        }
    }

    /// Hands a finished file to the person as a download.
    pub(crate) fn download(&mut self, name: &str, bytes: &[u8], mime: &str) {
        self.platform.download(name, bytes, mime);
        self.say(trf!("Downloaded {name}", name = name));
    }

    /// Adds the project to the recent list whenever the document's path changes (open, save as).
    fn track_recent(&mut self) {
        if self.session.path == self.recorded_path {
            return;
        }
        self.recorded_path = self.session.path.clone();
        if let Some(path) = self.recorded_path.clone() {
            self.recent.push(path);
            self.save_recent();
        }
    }

    /// Remembers a project's contents for "Open Recent" in a browser (a no-op elsewhere).
    pub(crate) fn remember_project(&mut self, name: &str, bytes: &[u8]) {
        if !self.platform.browser_files() {
            return;
        }
        if self.browser_recent.push(name, bytes) {
            self.save_browser_recent();
        }
    }

    pub(crate) fn save_browser_recent(&self) {
        self.platform.storage_set(browser_recent::STORAGE_KEY, &self.browser_recent.to_text());
    }

    fn forget_recent(&mut self, path: &std::path::Path) {
        self.recent.remove(path);
        self.save_recent();
    }

    fn save_shortcuts(&self) {
        if let Some(file) = self.platform.shortcuts_file() {
            self.shortcuts.save(&file);
        }
    }

    fn save_recent(&self) {
        if let Some(file) = self.platform.recent_file() {
            self.recent.save(&file);
        }
    }

    /// Asks before discarding unsaved work, then runs `action`.
    pub(crate) fn request(&mut self, action: PendingAction) {
        if self.session.dirty {
            self.pending_action = Some(action);
        } else {
            self.proceed(action);
        }
    }

    fn proceed(&mut self, action: PendingAction) {
        match action {
            PendingAction::New => {
                self.run("file.new", Value::Null);
                self.clear_recovery();
                self.autosave_revision = None;
                self.view = None;
            }
            PendingAction::Open if self.platform.browser_files() => self.begin_upload(UploadKind::Project),
            PendingAction::Open => {
                if let Some(path) = self.platform.pick_open("Open project", &["laserfoundry"]) {
                    self.run("file.open", serde_json::json!({ "path": path }));
                    self.clear_recovery();
                    self.autosave_revision = None;
                    self.view = None;
                }
            }
            PendingAction::OpenPath(path) => {
                if self.run("file.open", serde_json::json!({ "path": path })).is_some() {
                    self.clear_recovery();
                    self.autosave_revision = None;
                    self.view = None;
                } else {
                    // A file that is gone or unreadable should not stay on the menu.
                    self.forget_recent(&path);
                }
            }
            PendingAction::OpenStored(name) => match self.browser_recent.bytes(&name) {
                Some(bytes) if self.run("file.open", serde_json::json!({ "data_base64": laserfoundry_doc::base64::encode(&bytes) })).is_some() => {
                    self.clear_recovery();
                    self.autosave_revision = None;
                    self.view = None;
                    self.say(trf!("Opened {name}", name = name));
                }
                _ => {
                    // Damaged or gone: it should not stay on the menu.
                    self.browser_recent.remove(&name);
                    self.save_browser_recent();
                }
            },
            PendingAction::FromTemplate(path) => {
                if self.run("file.open", serde_json::json!({ "path": path, "template": true })).is_some() {
                    self.clear_recovery();
                    self.autosave_revision = None;
                    self.view = None;
                }
            }
            PendingAction::Close => {
                self.allow_close = true;
                self.clear_recovery();
            }
        }
    }

    /// The person's answer to "recover unsaved work?".
    fn answer_recovery(&mut self, recover: bool) {
        let Some(path) = self.pending_recovery.take() else { return };
        if recover {
            self.run("file.recover", serde_json::json!({ "path": path }));
            self.view = None;
            self.stale_backups = true;
        } else {
            self.clear_recovery();
        }
    }

    /// The person's answer to "save changes first?". Saving that does not complete (cancelled file
    /// dialog, failed write) keeps the question open: the work is never dropped by accident.
    fn answer_unsaved(&mut self, answer: Answer) {
        let Some(action) = self.pending_action.clone() else { return };
        match answer {
            Answer::Save => {
                if menus::save(self, false) {
                    self.pending_action = None;
                    self.proceed(action);
                }
            }
            Answer::Discard => {
                self.pending_action = None;
                self.proceed(action);
            }
            Answer::Cancel => self.pending_action = None,
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        shortcuts::window(self, ctx);
        history_ui::window(self, ctx);
        select_size_ui::window(self, ctx);
        font_ui::window(self, ctx);
        if self.pending_recovery.is_some() {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("recovery")).show(ctx, |ui| {
                ui.heading(tr("Recover unsaved work?"));
                ui.label(tr("LaserFoundry found work from a previous session that was never saved."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Recover")).clicked() {
                        choice = Some(true);
                    }
                    if ui.button(tr("Discard")).clicked() {
                        choice = Some(false);
                    }
                });
            });
            if let Some(recover) = choice {
                self.answer_recovery(recover);
            }
            return;
        }
        if self.pending_start.is_some() {
            let mut answer = None;
            egui::Modal::new(egui::Id::new("relative_start")).show(ctx, |ui| {
                ui.heading(tr("Start from where the head is?"));
                ui.label(tr("This job is placed from the head's position or your own origin, so LaserFoundry cannot check that it fits on the bed from there."));
                ui.label(tr("Make sure the head is where the job should begin and has room on every side, or turn on the controller's soft limits."));
                ui.checkbox(&mut self.relative_start_trusted, tr("Do not ask again in this session"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Start")).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        answer = Some(false);
                        self.relative_start_trusted = false;
                    }
                });
            });
            if let Some(go) = answer
                && let Some(program) = self.pending_start.take()
                && go
            {
                self.begin_job(program);
            }
            return;
        }
        let Some(action) = self.pending_action.clone() else { return };
        let name = self.session.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled".to_string(), |n| n.to_string_lossy().into_owned());
        let mut answer = None;
        egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.heading(tr("Unsaved changes"));
            ui.label(trf!("Save changes to {name} first?", name = name));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Save")).clicked() {
                    answer = Some(Answer::Save);
                }
                if ui.button(tr("Discard")).clicked() {
                    answer = Some(Answer::Discard);
                }
                if ui.button(tr("Cancel")).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        if let Some(answer) = answer {
            self.answer_unsaved(answer);
        }
        if action == PendingAction::Close && self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Runs a command, reporting failures in the status bar instead of dropping them.
    pub(crate) fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        match self.session.execute(id, &params) {
            Ok(v) => Some(v),
            Err(e) => {
                self.report_error(&e);
                None
            }
        }
    }

    /// Remembers characters the stroke font cannot draw so the inspector can say so.
    pub(crate) fn note_unsupported(&mut self, result: &Value) {
        let chars: Vec<String> =
            result["unsupported"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
        self.text_warning = (!chars.is_empty()).then(|| trf!("No glyph for: {chars} (drawn as a box)", chars = chars.join(" ")));
    }

    pub(crate) fn say(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), false));
    }

    pub(crate) fn report_error(&mut self, e: &EngineError) {
        self.status = Some((i18n::tr_message(&e.to_string()), true));
    }

    pub(crate) fn platform_preset_library(&self) -> Option<PathBuf> {
        self.platform.preset_library_file()
    }

    /// Merges the project's presets into the shared library file (names already there are kept).
    pub(crate) fn save_preset_library(&mut self, file: &std::path::Path) {
        let Some(v) = self.run("preset.export", Value::Null) else { return };
        let mut library = read_library(file);
        let known: Vec<String> = library.iter().filter_map(|p| p["name"].as_str().map(String::from)).collect();
        let mut added = 0;
        for preset in v["presets"].as_array().cloned().unwrap_or_default() {
            if preset["name"].as_str().is_some_and(|n| !known.iter().any(|k| k == n)) {
                library.push(preset);
                added += 1;
            }
        }
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&library).map(|text| std::fs::write(file, text)) {
            Ok(Ok(())) => self.say(trf!("{added} preset(s) added to your library", added = added)),
            _ => self.status = Some((tr("Could not write the preset library"), true)),
        }
    }

    /// Adds the library's presets to the project.
    pub(crate) fn load_preset_library(&mut self, file: &std::path::Path) {
        let library = read_library(file);
        if library.is_empty() {
            self.say(tr("Your preset library is empty"));
            return;
        }
        if let Some(v) = self.run("preset.import", serde_json::json!({ "presets": library })) {
            self.say(trf!("{count} preset(s) added from your library", count = v["added"]));
        }
    }

    pub(crate) fn templates_available(&self) -> bool {
        self.platform.templates_dir().is_some()
    }

    /// Template files, by name.
    pub(crate) fn templates(&self) -> Vec<(String, PathBuf)> {
        let Some(dir) = self.platform.templates_dir() else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut found: Vec<(String, PathBuf)> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "laserfoundry"))
            .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
            .collect();
        found.sort();
        found.truncate(100);
        found
    }

    /// Keeps the current project as a template under `name` (letters, digits, spaces, `-` and `_`).
    pub(crate) fn save_template(&mut self, name: &str) {
        let clean: String = name.trim().chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).take(60).collect();
        let Some(dir) = self.platform.templates_dir() else { return };
        if clean.is_empty() {
            self.status = Some((tr("Give the template a name"), true));
            return;
        }
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(format!("{clean}.laserfoundry"));
        // Saved as data, so the open project keeps its own path and unsaved state.
        let was_dirty = self.session.dirty;
        let Some(v) = self.session.execute("file.project_data", &serde_json::Value::Null).ok() else { return };
        let bytes = v["data_base64"].as_str().and_then(laserfoundry_doc::base64::decode).unwrap_or_default();
        self.session.dirty = was_dirty;
        match std::fs::write(&file, bytes) {
            Ok(()) => self.say(trf!("Saved template {clean}", clean = clean)),
            Err(e) => self.status = Some((trf!("Could not save the template: {e}", e = e), true)),
        }
    }

    pub(crate) fn platform_font_library_available(&self) -> bool {
        self.platform.font_library_available()
    }

    pub(crate) fn platform_library_fonts(&self) -> Vec<(String, String)> {
        self.platform.library_fonts()
    }

    pub(crate) fn platform_browser_files(&self) -> bool {
        self.platform.browser_files()
    }

    pub(crate) fn pick_open(&self, title: &str, exts: &[&str]) -> Option<PathBuf> {
        self.platform.pick_open(title, exts)
    }

    pub(crate) fn pick_save(&self, title: &str, name: &str, exts: &[&str]) -> Option<PathBuf> {
        self.platform.pick_save(title, name, exts)
    }

    /// Opens the selected machine (or the simulator) and keeps the connection until `disconnect`.
    pub(crate) fn connect(&mut self) {
        self.reconnect = None;
        if !self.platform.machine_available() {
            self.status = Some((tr("Connecting to a machine is not available in this build"), true));
            return;
        }
        let inline = self.platform.inline_link();
        let transport: Result<Box<dyn laserfoundry_device::Transport + Send>, String> = match &self.target {
            Target::Simulator => {
                let mut sim = SimulatedGrbl::new();
                if inline {
                    sim.set_non_blocking(true);
                } else {
                    sim.set_latency(std::time::Duration::from_millis(3));
                }
                Ok(Box::new(sim))
            }
            Target::Serial(_) if self.platform.serial_async() => {
                // The host asks the person to choose a device; the link starts when it is open.
                self.awaiting_serial = true;
                self.platform.request_serial();
                return;
            }
            Target::Serial(port) => self.platform.open_serial(port),
        };
        self.start_link(transport);
    }

    fn start_link(&mut self, transport: Result<Box<dyn laserfoundry_device::Transport + Send>, String>) {
        match transport {
            Ok(t) => {
                self.link = Some(if self.platform.inline_link() { Link::connect_inline(t) } else { Link::connect(t) });
                self.settings_asked = false;
                self.say(tr("Connected"));
            }
            Err(e) => self.status = Some((trf!("Could not open the machine: {e}", e = i18n::tr_message(&e.to_string())), true)),
        }
    }

    fn poll_serial(&mut self, ctx: &egui::Context) {
        if !self.awaiting_serial {
            return;
        }
        match self.platform.take_serial() {
            Some(result) => {
                self.awaiting_serial = false;
                self.start_link(result);
            }
            None => ctx.request_repaint_after(Duration::from_millis(200)),
        }
    }

    /// Dropping the link cancels any running job (soft reset) before the connection closes.
    pub(crate) fn disconnect(&mut self) {
        self.link = None;
        self.reconnect = None;
        self.move_head_mode = false;
        self.snap = None;
        self.last_job = None;
        self.say(tr("Disconnected"));
    }

    pub(crate) fn start_job(&mut self, program: String) {
        if self.link.is_none() {
            self.status = Some((tr("Connect to a machine (or the simulator) first"), true));
            return;
        }
        // A controller that is in an alarm, holding, homing or otherwise busy would refuse or garble the program.
        if let Some(state) = self.snap.as_ref().and_then(|s| s.status.as_ref()).map(|s| s.state)
            && let Some(why) = not_ready(state)
        {
            self.status = Some((why, true));
            return;
        }
        // Without laser mode ($32=1) GRBL leaves the beam on during travel moves of a laser program.
        if self
            .snap
            .as_ref()
            .is_some_and(|s| settings_ui::advice(&s.settings, &self.session.doc.machine).contains(&settings_ui::Advice::LaserModeOff))
        {
            self.status = Some((tr("Laser mode ($32) is off on this controller: turn it on in the machine settings before running a job"), true));
            return;
        }
        // From the head's position or a user origin the job's place on the bed is not known to the program, so
        // the bed cannot be checked: make the person confirm.
        if self.session.doc.job_origin.start_from != laserfoundry_doc::StartFrom::Absolute && !self.relative_start_trusted {
            self.pending_start = Some(program);
            return;
        }
        self.begin_job(program);
    }

    /// Hands the program to the machine link.
    pub(crate) fn begin_job(&mut self, program: String) {
        let Some(link) = &self.link else { return };
        let kept: std::sync::Arc<str> = program.as_str().into();
        if let Err(e) = link.start_job(program) {
            self.status = Some((i18n::tr_message(&e.to_string()), true));
        } else {
            self.job_program = Some(kept);
            self.forget_interrupted();
        }
    }

    /// Drops the note about a job an earlier session left unfinished.
    pub(crate) fn forget_interrupted(&mut self) {
        self.interrupted = None;
        if let Some(path) = self.platform.job_position_file() {
            job_position::clear(&path);
        }
    }

    /// Keeps the saved job position current while a job runs, and removes it when the job ends cleanly.
    fn record_job_position(&mut self, snap: &Snapshot) {
        let Some(path) = self.platform.job_position_file() else { return };
        let write = |program: &str, acknowledged: usize| {
            let total = program.lines().count();
            let line = laserfoundry_device::restart_line(program, acknowledged).min(total.max(1));
            job_position::save(&path, &job_position::JobPosition { hash: job_position::fingerprint(program), line, total_lines: total });
        };
        match &snap.job {
            JobState::Running | JobState::Paused => {
                let (Some(p), Some(program)) = (snap.progress, self.job_program.clone()) else { return };
                if p.acknowledged > 0 && self.last_position_write.elapsed() >= Duration::from_secs(job_position::WRITE_EVERY_SECS) {
                    self.last_position_write = Instant::now();
                    write(&program, p.acknowledged);
                }
            }
            JobState::Finished(Ok(_)) => {
                if self.job_program.take().is_some() {
                    job_position::clear(&path);
                }
            }
            JobState::Finished(Err(_)) => {
                if let (Some(program), Some(p)) = (self.job_program.take(), snap.progress)
                    && p.acknowledged > 0
                {
                    write(&program, p.acknowledged);
                }
            }
            _ => {}
        }
    }

    /// Runs a link command and shows a refusal in the status bar.
    pub(crate) fn link_do(&mut self, f: impl FnOnce(&Link) -> Result<(), laserfoundry_device::LinkError>) {
        let Some(link) = &self.link else {
            self.status = Some((tr("Not connected"), true));
            return;
        };
        if let Err(e) = f(link) {
            self.status = Some((i18n::tr_message(&e.to_string()), true));
        }
    }

    pub(crate) fn job_active(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| matches!(s.job, JobState::Running | JobState::Paused))
    }

    /// Retries a lost serial connection with growing waits. A reconnected machine is idle: a job
    /// that was running is never resumed.
    fn reconnect_tick(&mut self, ctx: &egui::Context) {
        let Some(state) = &self.reconnect else { return };
        let (attempts, next) = (state.attempts, state.next);
        if Instant::now() < next {
            ctx.request_repaint_after(next - Instant::now());
            return;
        }
        let Target::Serial(port) = self.target.clone() else {
            self.reconnect = None;
            return;
        };
        match self.platform.open_serial(&port) {
            Ok(t) => {
                self.link = Some(Link::connect(t));
                self.settings_asked = false;
                self.reconnect = None;
                self.say(tr("Reconnected. Nothing is running; start the job again if you want to continue."));
            }
            Err(e) => {
                let attempts = attempts + 1;
                if attempts >= RECONNECT_ATTEMPTS {
                    self.reconnect = None;
                    self.status = Some((trf!("Could not reconnect to {port}: {e}", port = port, e = i18n::tr_message(&e.to_string())), true));
                } else {
                    let wait = (self.reconnect_wait * 2u32.saturating_pow(attempts)).min(RECONNECT_MAX_WAIT);
                    self.reconnect = Some(Reconnect { attempts, next: Instant::now() + wait });
                    ctx.request_repaint_after(wait);
                }
            }
        }
    }

    fn poll_link(&mut self, ctx: &egui::Context) {
        self.reconnect_tick(ctx);
        let Some(link) = &self.link else { return };
        // The worker updates continuously; poll at a modest rate rather than every frame forever.
        // An inline link only works when ticked, so it needs frames more often.
        ctx.request_repaint_after(std::time::Duration::from_millis(if self.platform.inline_link() { 30 } else { 100 }));
        link.tick();
        let snap = link.snapshot();
        if !snap.connected {
            let had_job = matches!(snap.job, JobState::Running | JobState::Paused | JobState::Finished(Err(_)));
            self.link = None;
            self.snap = None;
            self.move_head_mode = false;
            if self.auto_reconnect && matches!(self.target, Target::Serial(_)) && !self.platform.serial_async() {
                self.reconnect = Some(Reconnect { attempts: 0, next: Instant::now() + self.reconnect_wait });
                let note = if had_job { tr(" The job was stopped and will not resume by itself.") } else { String::new() };
                self.status = Some((trf!("The machine connection was lost; trying to reconnect.{note}", note = note), true));
            } else {
                self.status = Some((tr("The machine connection was lost"), true));
            }
            return;
        }
        if !self.settings_asked && snap.status.is_some() {
            // Once the controller answers, learn its settings: a job is refused when laser mode is off.
            self.settings_asked = true;
            let _ = link.request_settings();
        }
        if self.last_job.as_ref() != Some(&snap.job) {
            if let JobState::Finished(result) = &snap.job {
                self.status = Some(match result {
                    Ok(lines) => (trf!("Job finished: {lines} lines", lines = lines), false),
                    Err(e) => (trf!("Job stopped: {e}", e = i18n::tr_message(&e.to_string())), true),
                });
            }
            self.last_job = Some(snap.job.clone());
        }
        self.record_job_position(&snap);
        self.snap = Some(snap);
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, Modifiers};
        use shortcuts::Action;
        let (alias_redo, delete) = ctx.input_mut(|i| {
            (i.consume_key(Modifiers::COMMAND, Key::Y), i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace))
        });
        let mut pressed = self.shortcuts.pressed(ctx);
        if alias_redo {
            pressed.push(Action::Redo);
        }
        for action in pressed {
            self.run(action.command(), Value::Null);
        }
        if delete && self.tool == Tool::Nodes && self.node_sel.is_some() {
            nodes_ui::delete_selected(self);
        } else if delete && !self.session.selection.is_empty() {
            self.run("edit.delete", Value::Null);
        }
        let (enter, escape) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape)));
        if enter && self.pen.is_some() {
            nodes_ui::finish(self, false);
        }
        if escape {
            self.tracked.clear();
            self.track_candidate = None;
            if self.move_head_mode {
                self.move_head_mode = false;
            } else if self.crop_mode {
                self.crop_mode = false;
                self.crop_drag = None;
            } else if self.marks.pick.is_some() {
                self.marks.pick = None;
            } else if self.pen.is_some() || self.node_drag.is_some() {
                nodes_ui::cancel(self);
            } else if self.tool != Tool::Select {
                self.tool = Tool::Select;
            }
        }
        let nudge = ctx.input_mut(|i| {
            let step = if i.modifiers.shift { 10.0 } else { 1.0 };
            let mut d = (0.0, 0.0);
            for (key, dx, dy) in [(Key::ArrowLeft, -step, 0.0), (Key::ArrowRight, step, 0.0), (Key::ArrowUp, 0.0, -step), (Key::ArrowDown, 0.0, step)]
            {
                if i.consume_key(Modifiers::NONE, key) || i.consume_key(Modifiers::SHIFT, key) {
                    d = (d.0 + dx, d.1 + dy);
                }
            }
            d
        });
        if nudge != (0.0, 0.0) && !self.session.selection.is_empty() {
            self.run("edit.move", serde_json::json!({ "dx": nudge.0, "dy": nudge.1 }));
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match &self.status {
            Some((text, true)) => {
                ui.colored_label(self.tokens.error, text);
            }
            Some((text, false)) => {
                ui.colored_label(self.tokens.ok, text);
            }
            None => {
                ui.label(tr("Ready"));
            }
        });
    }
}

#[cfg(test)]
mod tests;

pub(crate) enum Answer {
    Save,
    Discard,
    Cancel,
}

/// `name — LaserFoundry`, with `*` while there are unsaved changes.
pub(crate) fn window_title(session: &Session) -> String {
    let name = session.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled".to_string(), |n| n.to_string_lossy().into_owned());
    format!("{}{name} — LaserFoundry", if session.dirty { "*" } else { "" })
}

/// The presets in a library file; a missing, oversized or damaged file is an empty library.
fn read_library(file: &std::path::Path) -> Vec<Value> {
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    if std::fs::metadata(file).map_or(true, |m| m.len() > MAX_BYTES) {
        return Vec::new();
    }
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok()).unwrap_or_default()
}

/// Why a job cannot start while the controller reports `state` (`None` when it can).
fn not_ready(state: laserfoundry_device::MachineState) -> Option<String> {
    use laserfoundry_device::MachineState as M;
    Some(match state {
        M::Idle | M::Unknown => return None,
        M::Alarm => tr("The machine is in an alarm state: use Unlock ($X) or Home ($H) first"),
        M::Hold => tr("The machine is holding (paused): resume it or stop it first"),
        M::Door => tr("The machine's safety door is open"),
        M::Home => tr("The machine is homing: wait until it has finished"),
        M::Run | M::Jog => tr("The machine is still moving: wait until it is idle"),
        M::Check => tr("The machine is in check mode ($C): turn it off first"),
        M::Sleep => tr("The machine is asleep: wake it first"),
    })
}
