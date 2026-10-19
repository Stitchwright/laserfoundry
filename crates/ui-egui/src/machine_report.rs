//! A plain-text report about the machine link, to save and send along with a problem: what the program and
//! the document assume, what the controller reported, and the console as it stands. Nothing private goes in
//! (no file names, no account), so it is safe to paste into a bug report.

use std::fmt::Write;

use crate::LightFoundryApp;

/// The most console lines kept (the newest).
const MAX_CONSOLE_LINES: usize = 400;

/// The report as text.
pub(crate) fn report(app: &LightFoundryApp) -> String {
    let m = &app.session.doc.machine;
    let mut out = String::new();
    let _ = writeln!(out, "LightFoundry machine report");
    let _ = writeln!(out, "Version: {}  System: {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    let target = match &app.target {
        crate::Target::Simulator => "simulator".to_string(),
        crate::Target::Serial(port) => format!("serial {port}"),
    };
    let _ = writeln!(out, "Connection: {target}  Connected: {}", app.snap.as_ref().is_some_and(|s| s.connected));
    let _ = writeln!(
        out,
        "Document machine: bed {} x {} mm, origin {:?}, dialect {:?}, S max {}, dynamic power {}, acceleration {} mm/s2",
        m.bed_width, m.bed_height, m.origin, m.dialect, m.max_s, m.dynamic_power, m.acceleration
    );
    let _ = writeln!(out, "Job origin: {:?}", app.session.doc.job_origin);
    if let Some(s) = &app.snap {
        if let Some(status) = &s.status {
            let _ = writeln!(
                out,
                "Controller state: {:?}  Position: {:?}  (work coordinates: {})",
                status.state, status.position, status.position_is_work
            );
        }
        let _ = writeln!(out, "Job: {:?}", s.job);
        let _ = writeln!(out, "\nController settings ($$):");
        if s.settings.is_empty() {
            let _ = writeln!(out, "  (none read yet)");
        }
        for (id, value) in &s.settings {
            let _ = writeln!(out, "  ${id}={value}");
        }
        let _ = writeln!(out, "\nConsole (newest last):");
        let skip = s.console.len().saturating_sub(MAX_CONSOLE_LINES);
        for line in s.console.iter().skip(skip) {
            let _ = writeln!(out, "  {}", line.chars().take(300).collect::<String>());
        }
    } else {
        let _ = writeln!(out, "\n(not connected: no controller information)");
    }
    out
}

impl LightFoundryApp {
    /// Saves the machine report to a file the person chooses (a download in a browser).
    pub(crate) fn save_machine_report(&mut self) {
        let text = report(self);
        if self.platform.browser_files() {
            self.download("lightfoundry-machine-report.txt", text.as_bytes(), "text/plain");
            return;
        }
        let Some(path) = self.pick_save("Save machine report", "lightfoundry-machine-report.txt", &["txt"]) else { return };
        match std::fs::write(&path, text) {
            Ok(()) => self.say(crate::i18n::trf!("Saved the machine report to {path}", path = path.display())),
            Err(e) => self.status = Some((crate::i18n::trf!("Could not save the report: {e}", e = e), true)),
        }
    }
}
