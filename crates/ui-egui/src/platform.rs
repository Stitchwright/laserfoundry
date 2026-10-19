use std::path::PathBuf;

use laserfoundry_device::Transport;

/// A file the person picked in a browser-style host: its name and contents.
#[derive(Debug, Clone, PartialEq)]
pub struct Upload {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// What the host operating system provides. Keeping this behind a trait keeps file dialogs and
/// serial ports out of the UI crate (and lets tests and the web build run without them).
pub trait Platform {
    fn pick_open(&self, title: &str, extensions: &[&str]) -> Option<PathBuf>;
    fn pick_save(&self, title: &str, default_name: &str, extensions: &[&str]) -> Option<PathBuf>;
    /// Where the autosave copy lives, if this platform has somewhere to keep one.
    fn recovery_file(&self) -> Option<PathBuf> {
        None
    }

    /// Removes the autosave copy (the work was saved or knowingly discarded).
    fn clear_recovery(&self) {}

    /// Where the position of a job in progress is kept, so it can be resumed after a power cut.
    fn job_position_file(&self) -> Option<PathBuf> {
        None
    }

    /// Where the chosen translation (a JSON catalog, see `i18n`) is kept, if this platform has somewhere to keep it.
    fn translation_file(&self) -> Option<PathBuf> {
        None
    }

    /// Where presets shared between projects are kept, if this platform has somewhere to keep them.
    fn preset_library_file(&self) -> Option<PathBuf> {
        None
    }

    /// A folder of project templates (`.laserfoundry` files), if this platform has one.
    fn templates_dir(&self) -> Option<PathBuf> {
        None
    }

    /// Where the list of recent projects is kept, if this platform has somewhere to keep it.
    fn recent_file(&self) -> Option<PathBuf> {
        None
    }

    /// Small text the host keeps between visits for hosts with no files of their own (the browser's
    /// local storage). `None` when nothing is stored or storage is unavailable.
    fn storage_get(&self, _key: &str) -> Option<String> {
        None
    }

    /// Whether this host can send the AI and account requests (the desktop app can; the hosted browser build
    /// cannot until the page's content security policy names a service).
    fn http_available(&self) -> bool {
        false
    }

    /// Starts sending `request` in the background. False when this host cannot, or one is already running.
    fn start_http(&self, _request: laserfoundry_ai::HttpRequest) -> bool {
        false
    }

    /// Once, after a started request has finished: the HTTP status and body, or why nothing came back.
    fn take_http(&self) -> Option<Result<(u16, String), String>> {
        None
    }

    /// Reads something secret the host keeps for the app (an account's session token, an API key). By default
    /// the same small storage as [`storage_get`](Platform::storage_get); the desktop host keeps it in a file
    /// only its owner can read.
    fn secret_get(&self, key: &str) -> Option<String> {
        self.storage_get(key)
    }

    fn secret_set(&self, key: &str, value: &str) {
        self.storage_set(key, value);
    }

    /// Where a log of every AI request and answer is kept, so the person can see exactly what was sent. `None`
    /// where the host has nowhere to keep one (the browser).
    fn ai_log_file(&self) -> Option<PathBuf> {
        None
    }

    /// Called every frame: whether leaving now would lose work or interrupt a running job. A browser host
    /// asks the person to confirm before the page is closed or reloaded; other hosts ignore it.
    fn set_leave_warning(&self, _on: bool) {}

    /// Stores `value` under `key`; failures (a full or blocked storage) are the host's to log.
    fn storage_set(&self, _key: &str, _value: &str) {}

    /// Whether this host may download the font library (the desktop app can; the hosted browser build
    /// never does: it takes nothing from third-party servers and has no folder to keep fonts in).
    fn font_library_available(&self) -> bool {
        false
    }

    /// Starts downloading the font library in the background (`everything` adds the large
    /// Ink/Stitch collection). False when this host cannot, or a download is already running.
    fn start_font_library_download(&self, _everything: bool) -> bool {
        false
    }

    /// Once, after a started download has finished: a summary, or the reason it failed.
    fn take_font_library_result(&self) -> Option<Result<String, String>> {
        None
    }

    /// Whether the large Ink/Stitch collection can be offered too (the hosted browser build carries
    /// only the Foundry fonts).
    fn font_library_everything_available(&self) -> bool {
        true
    }

    /// The fonts ready to use for text as (display name, value for the text command's `font`), sorted.
    /// By default, the library folders found in [`font_library_dir`](Platform::font_library_dir).
    fn library_fonts(&self) -> Vec<(String, String)> {
        self.font_library_dir()
            .map(|dir| crate::panels::library_fonts(&dir).into_iter().map(|(name, path)| (name, path.display().to_string())).collect())
            .unwrap_or_default()
    }

    /// Where the font library is kept, if this host has such a folder.
    fn font_library_dir(&self) -> Option<PathBuf> {
        None
    }

    /// Where the keyboard shortcut bindings are kept, if this platform has somewhere to keep them.
    fn shortcuts_file(&self) -> Option<PathBuf> {
        None
    }

    /// Browser-style hosts cannot hand out file paths: files arrive as bytes after the person has
    /// chosen them ([`request_upload`](Platform::request_upload), then
    /// [`take_upload`](Platform::take_upload)) and leave as downloads.
    fn browser_files(&self) -> bool {
        false
    }

    /// Asks the person to choose a file with one of these extensions.
    fn request_upload(&self, _extensions: &[&str]) {}

    /// The file chosen after a request, once it has been read.
    fn take_upload(&self) -> Option<Upload> {
        None
    }

    /// Offers `bytes` to the person as a downloaded file called `name`.
    fn download(&self, _name: &str, _bytes: &[u8], _mime: &str) {}

    /// Whether this platform can run a machine link at all (it needs threads, which the browser
    /// build does not have).
    fn machine_available(&self) -> bool {
        true
    }

    /// Hosts without threads (the browser) run the machine link on the caller's thread, driven by
    /// `Link::tick` each frame.
    fn inline_link(&self) -> bool {
        false
    }

    /// Whether the serial port is chosen and opened asynchronously by the host (the browser asks
    /// the person to pick a device): call [`request_serial`](Platform::request_serial), then poll
    /// [`take_serial`](Platform::take_serial).
    fn serial_async(&self) -> bool {
        false
    }

    fn request_serial(&self) {}

    fn take_serial(&self) -> Option<Result<Box<dyn Transport + Send>, String>> {
        None
    }

    fn serial_ports(&self) -> Vec<String>;
    fn open_serial(&self, port: &str) -> Result<Box<dyn Transport + Send>, String>;
}

/// No dialogs, no serial ports: only the built-in simulator is available.
pub struct NoPlatform;

impl Platform for NoPlatform {
    fn pick_open(&self, _: &str, _: &[&str]) -> Option<PathBuf> {
        None
    }

    fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<PathBuf> {
        None
    }

    fn serial_ports(&self) -> Vec<String> {
        Vec::new()
    }

    fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
        Err("serial ports are not available on this platform".into())
    }
}
