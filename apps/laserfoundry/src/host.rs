use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, mpsc};

use laserfoundry_device::Transport;
use laserfoundry_mcp::link::LinkServer;
use laserfoundry_ui_egui::{LaserFoundryApp, Platform};

pub struct Host {
    pub app: LaserFoundryApp,
    /// Present only when the person switched agent access on; closing the app closes it.
    pub agent_link: Option<LinkServer>,
}

impl eframe::App for Host {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(link) = &mut self.agent_link {
            link.serve_pending(&mut self.app.session);
        }
        self.app.ui(ui);
    }
}

/// An AI or account request's answer: the HTTP status and body, or why none came.
type HttpAnswer = Result<(u16, String), String>;

/// Native file dialogs and serial ports.
#[derive(Default)]
pub struct DesktopPlatform {
    /// The font library download in progress, if any: its result arrives here.
    font_job: Mutex<Option<mpsc::Receiver<Result<String, String>>>>,
    /// The AI or account request in flight, if any: its answer arrives here.
    http_job: Mutex<Option<mpsc::Receiver<HttpAnswer>>>,
}

impl Platform for DesktopPlatform {
    fn recovery_file(&self) -> Option<PathBuf> {
        Some(data_dir()?.join("LaserFoundry").join("recovery.laserfoundry"))
    }

    fn job_position_file(&self) -> Option<PathBuf> {
        Some(data_dir()?.join("LaserFoundry").join("job-position.json"))
    }

    fn translation_file(&self) -> Option<PathBuf> {
        laserfoundry_i18n::default_file()
    }

    fn preset_library_file(&self) -> Option<PathBuf> {
        Some(data_dir()?.join("LaserFoundry").join("presets.json"))
    }

    fn templates_dir(&self) -> Option<PathBuf> {
        Some(data_dir()?.join("LaserFoundry").join("templates"))
    }

    fn recent_file(&self) -> Option<PathBuf> {
        Some(data_dir()?.join("LaserFoundry").join("recent.json"))
    }

    fn shortcuts_file(&self) -> Option<PathBuf> {
        Some(data_dir()?.join("LaserFoundry").join("shortcuts.json"))
    }

    fn font_library_available(&self) -> bool {
        laserfoundry_fontlib::default_dir().is_some()
    }

    fn font_library_dir(&self) -> Option<PathBuf> {
        laserfoundry_fontlib::default_dir()
    }

    fn start_font_library_download(&self, everything: bool) -> bool {
        let (Some(dest), Ok(mut job)) = (laserfoundry_fontlib::default_dir(), self.font_job.lock()) else { return false };
        if job.is_some() {
            return false;
        }
        let (sender, receiver) = mpsc::channel();
        *job = Some(receiver);
        let set = if everything { laserfoundry_fontlib::Set::Everything } else { laserfoundry_fontlib::Set::Foundry };
        std::thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            let result = laserfoundry_fontlib::download(&dest, &laserfoundry_fontlib::Source::default(), set, &cancel, &mut |_, _| {})
                .map(|s| {
                    format!(
                        "Font library: {} downloaded, {} already there, in {}. The Foundry fonts are under Library fonts in the text inspector.",
                        s.fetched,
                        s.kept,
                        dest.display()
                    )
                })
                .map_err(|e| e.to_string());
            let _ = sender.send(result);
        });
        true
    }

    fn take_font_library_result(&self) -> Option<Result<String, String>> {
        let mut job = self.font_job.lock().ok()?;
        match job.as_ref()?.try_recv() {
            Ok(result) => {
                *job = None;
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                *job = None;
                Some(Err("the font download stopped unexpectedly".into()))
            }
        }
    }

    fn http_available(&self) -> bool {
        true
    }

    fn start_http(&self, request: laserfoundry_ai::HttpRequest) -> bool {
        let Ok(mut job) = self.http_job.lock() else { return false };
        if job.is_some() {
            return false;
        }
        let (sender, receiver) = mpsc::channel();
        *job = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(crate::net::send(&request));
        });
        true
    }

    fn take_http(&self) -> Option<Result<(u16, String), String>> {
        let mut job = self.http_job.lock().ok()?;
        match job.as_ref()?.try_recv() {
            Ok(result) => {
                *job = None;
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                *job = None;
                Some(Err("the request stopped unexpectedly".into()))
            }
        }
    }

    fn ai_log_file(&self) -> Option<PathBuf> {
        Some(data_dir()?.join("LaserFoundry").join("ai-log.jsonl"))
    }

    fn secret_get(&self, key: &str) -> Option<String> {
        crate::net::secret_get(&data_dir()?.join("LaserFoundry").join("secrets.json"), key)
    }

    fn secret_set(&self, key: &str, value: &str) {
        let Some(path) = data_dir().map(|d| d.join("LaserFoundry").join("secrets.json")) else { return };
        if let Err(e) = crate::net::secret_set(&path, key, value) {
            log::warn!("could not keep the account settings: {e}");
        }
    }

    fn clear_recovery(&self) {
        if let Some(path) = self.recovery_file()
            && let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not remove {}: {e}", path.display());
        }
    }

    fn pick_open(&self, title: &str, extensions: &[&str]) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).add_filter("Supported files", extensions).pick_file()
    }

    fn pick_save(&self, title: &str, default_name: &str, extensions: &[&str]) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).set_file_name(default_name).add_filter("File", extensions).save_file()
    }

    #[cfg(feature = "serial")]
    fn serial_ports(&self) -> Vec<String> {
        laserfoundry_device::serial::list_ports()
    }

    #[cfg(not(feature = "serial"))]
    fn serial_ports(&self) -> Vec<String> {
        Vec::new()
    }

    #[cfg(feature = "serial")]
    fn open_serial(&self, port: &str) -> Result<Box<dyn Transport + Send>, String> {
        let t = laserfoundry_device::serial::SerialTransport::open(port, laserfoundry_device::serial::DEFAULT_BAUD).map_err(|e| e.to_string())?;
        Ok(Box::new(t))
    }

    #[cfg(not(feature = "serial"))]
    fn open_serial(&self, _port: &str) -> Result<Box<dyn Transport + Send>, String> {
        Err("this build has no serial support".into())
    }
}

/// The per-user data folder: `$XDG_DATA_HOME` or `~/.local/share` on Linux, `~/Library/Application
/// Support` on macOS, `%APPDATA%` on Windows.
fn data_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
    }
}

/// Makes the app's own data folder (autosaves, recent projects, presets, the agent link file) readable by its
/// owner only, so other people on a shared computer cannot read a person's designs. A failure is logged and
/// the app carries on: the files inside are still written.
pub fn make_data_folder_private() {
    let Some(dir) = data_dir().map(|d| d.join("LaserFoundry")) else { return };
    if let Err(e) = make_private(&dir) {
        log::warn!("could not make {} private: {e}", dir.display());
    }
}

/// Creates `dir` if needed and, where the system has file modes, limits it to its owner.
fn make_private(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_becomes_owner_only() {
        let dir = std::env::temp_dir().join(format!("lf-private-{}", std::process::id())).join("LaserFoundry");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        make_private(&dir).unwrap();
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&dir).unwrap().permissions());
        assert_eq!(mode & 0o077, 0, "{mode:o}");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
