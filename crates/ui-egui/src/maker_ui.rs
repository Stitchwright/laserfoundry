//! MakerFoundry: opening a design from one of its pages, and publishing the open drawing to it.
//!
//! * **Open.** MakerFoundry's "Open in LaserFoundry" button links to the web app with `?open=<the design file's address>`. The
//!   host hands that address over ([`Platform::startup_link`](crate::Platform::startup_link)); it is checked strictly
//!   ([`maker::open_link`]), fetched without cookies, and the file opens as if it had been dropped on the page.
//! * **Publish.** The drawing goes to MakerFoundry as an SVG, signed with the person's Foundry Account. MakerFoundry keeps it
//!   as a draft and the person's browser opens its page, where the title, licence and price are set: publishing itself
//!   always happens on the site.
//!
//! The requests and the checks are in `laserfoundry_ai::maker`; this is the windows and the hand-offs to the host.

use std::time::Duration;

use laserfoundry_ai::{AiError, maker};
use serde_json::json;

use crate::LaserFoundryApp;
use crate::ai_ui::{Pending, Window};
use crate::i18n::{self, tr, trf};

/// What the MakerFoundry link is doing.
#[derive(Default)]
pub(crate) struct MakerState {
    /// The page's own link has been looked at (once, at start).
    link_read: bool,
    /// The design being fetched, with the checked address it came from.
    opening: Option<maker::OpenLink>,
}

impl LaserFoundryApp {
    /// Whether this host can talk to MakerFoundry (the same ability as the account service).
    pub(crate) fn maker_available(&self) -> bool {
        self.platform.http_available()
    }

    /// Looks at the link the page was opened with, once, and follows a design fetch to its end. Call every frame.
    pub(crate) fn poll_maker(&mut self, ctx: &egui::Context) {
        if !std::mem::replace(&mut self.maker.link_read, true)
            && let Some(value) = self.platform.startup_link()
        {
            self.open_from_link(&value);
        }
        self.poll_fetch(ctx);
    }

    fn open_from_link(&mut self, value: &str) {
        match maker::open_link(value) {
            Err(e) => self.open_failed(&e),
            // the address fetched is the checked one, never the text that came in
            Ok(link) if self.platform.start_fetch(&link.url, maker::MAX_OPEN_BYTES) => {
                self.say(tr("Opening the design from MakerFoundry…"));
                self.maker.opening = Some(link);
            }
            Ok(_) => self.status = Some((tr("This page can't open designs from a MakerFoundry link."), true)),
        }
    }

    fn poll_fetch(&mut self, ctx: &egui::Context) {
        let Some(link) = self.maker.opening.clone() else { return };
        let Some(result) = self.platform.take_fetch() else {
            ctx.request_repaint_after(Duration::from_millis(200));
            return;
        };
        self.maker.opening = None;
        let fetched = match result {
            Ok(fetched) => fetched,
            Err(e) => return self.open_failed(&AiError::Network(e)),
        };
        if fetched.status != 200 {
            return self.open_failed(&maker::open_failure(fetched.status, &fetched.bytes));
        }
        if fetched.bytes.len() > maker::MAX_OPEN_BYTES {
            return self.open_failed(&AiError::LinkTooBig);
        }
        match maker::file_name(&link, fetched.content_disposition.as_deref()) {
            Ok(name) => self.open_design(&name, &fetched.bytes),
            Err(e) => self.open_failed(&e),
        }
    }

    /// Opens a fetched design as a dropped file of the same name would be (the name was checked to be an SVG or DXF).
    fn open_design(&mut self, name: &str, bytes: &[u8]) {
        let id = if name.to_ascii_lowercase().ends_with(".dxf") { "file.import_dxf" } else { "file.import_svg" };
        if self.run(id, json!({ "data": String::from_utf8_lossy(bytes) })).is_some() {
            self.say(trf!("Opened {name}", name = name));
        }
    }

    fn open_failed(&mut self, e: &AiError) {
        self.status = Some((i18n::tr_message(&e.to_string()), true));
    }

    /// The file name a published drawing gets: the project's name, as a plain `.svg` name.
    fn publish_file_name(&self) -> String {
        let stem = self.session.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let plain: String = stem.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).take(60).collect();
        let plain = plain.trim_matches(['_', '.', ' ']);
        format!("{}.svg", if plain.is_empty() { "design" } else { plain })
    }

    pub(crate) fn publish_window(&mut self, ui: &mut egui::Ui) {
        ui.label(tr("Send this drawing to MakerFoundry as a draft. You choose the title, licence and price on MakerFoundry, and nothing is public until you publish it there."));
        ui.weak(trf!(
            "It is sent as {name} (an SVG with your layers' colours) to {server}.",
            name = self.publish_file_name(),
            server = maker::DEFAULT_SERVER
        ));
        if self.ai.signed_in() {
            let busy = self.ai.busy();
            if ui.add_enabled(!busy, egui::Button::new(tr("Send to MakerFoundry"))).clicked() {
                self.start_publish();
            }
        } else {
            ui.horizontal(|ui| {
                ui.colored_label(self.tokens.error, tr("Not signed in."));
                if ui.small_button(tr("Foundry Account…")).clicked() {
                    self.ai.window = Some(Window::Account);
                }
            });
            ui.weak(tr("MakerFoundry uses your Foundry Account, the same one as the AI features."));
        }
        self.note_row(ui);
    }

    /// Sends the drawing, as an SVG, to MakerFoundry; [`publish_answered`](Self::publish_answered) reads the answer.
    pub(crate) fn start_publish(&mut self) {
        let Some(token) = self.ai.token.clone() else {
            self.ai_error(&AiError::SignIn);
            return;
        };
        if self.session.doc.shapes.is_empty() {
            self.ai.note = Some((tr("Draw something first: there is nothing to send."), true));
            return;
        }
        let Some(exported) = self.run("file.export_svg", json!({ "include": true })) else { return };
        let Some(svg) = exported["svg"].as_str() else { return };
        let file = laserfoundry_doc::base64::encode(svg.as_bytes());
        let app = format!("LaserFoundry {}", env!("CARGO_PKG_VERSION"));
        let request = maker::publish(maker::DEFAULT_SERVER, &token, &self.publish_file_name(), &file, &app);
        self.send_ai(Pending::Publish, request);
    }

    pub(crate) fn publish_answered(&mut self, status: u16, body: &str) {
        match maker::read_published(maker::DEFAULT_SERVER, status, body) {
            Ok(draft) => {
                // the person's browser opens the draft's page (checked to be on MakerFoundry)
                self.pending_url = Some(draft.url);
                self.ai.note = Some((
                    tr("Saved as a draft on MakerFoundry. Its page is opening in your browser: add the title, licence and price there."),
                    false,
                ));
            }
            Err(e) => self.ai_error(&e),
        }
    }
}

#[cfg(test)]
mod tests;
