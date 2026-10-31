//! The AI features and the account: Design from Prompt, Redraw Image, Ask About Lasering, Check My Cut and
//! the account window (sign in, credits, buying more). The requests and replies are `laserfoundry-ai`'s; the
//! host sends them ([`Platform::start_http`]); what changes in the document goes through engine commands.

use laserfoundry_ai::art::{ArtRequest, Use};
use laserfoundry_ai::ask::Turn;
use laserfoundry_ai::check::{Check, CheckRequest};
use laserfoundry_ai::cloud::{self, Account};
use laserfoundry_ai::{AiError, Auth, HttpRequest, Picture};
use serde_json::{Value, json};

use crate::LaserFoundryApp;
use crate::i18n::{self, tr, trf};

/// Where the account and key are kept (see [`Platform::secret_set`](crate::Platform::secret_set)).
const SECRET_KEY: &str = "laserfoundry.ai";

/// Which window is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Window {
    Account,
    Design,
    Redraw,
    Ask,
    Check,
    /// Send the open drawing to MakerFoundry (`maker_ui`).
    Publish,
}

/// Which window a picture being chosen is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PictureFor {
    Redraw,
    Check,
}

/// What the request in flight is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    SignIn,
    SignUp,
    SignOut,
    Account,
    Checkout,
    Art,
    Ask,
    Check,
    Publish,
}

/// Everything the AI windows remember.
#[derive(Default)]
pub(crate) struct AiState {
    pub(crate) window: Option<Window>,
    loaded: bool,
    // account
    server: String,
    email: String,
    password: String,
    key: String,
    use_key: bool,
    pub(crate) token: Option<String>,
    account: Option<Account>,
    pub(crate) pending: Option<Pending>,
    /// The person agreed to what is sent (kept with the account settings).
    consented: bool,
    /// Features the person switched off (indexed like [`Window::feature`]); all are on by default.
    off: [bool; 4],
    /// What is being logged about the request in flight (written when its answer comes).
    logging: Option<LogEntry>,
    /// A request waiting for the person's agreement before it is sent the first time.
    awaiting_consent: Option<(Pending, HttpRequest)>,
    /// A line under the window's buttons: what happened, and whether it is an error.
    pub(crate) note: Option<(String, bool)>,
    // Design from Prompt and Redraw Image
    prompt: String,
    size_mm: f64,
    use_for: Use,
    picture: Option<(String, Picture)>,
    /// Which window the picture being chosen (in a browser, not yet arrived) is for.
    picture_for: Option<PictureFor>,
    // Ask About Lasering
    question: String,
    turns: Vec<Turn>,
    // Check My Cut
    check_notes: String,
    photos: Vec<(String, Picture)>,
    check: Option<Check>,
}

/// One request's record in the log of what was sent.
struct LogEntry {
    feature: &'static str,
    destination: String,
    request: Value,
}

impl Window {
    /// Which of the four features a window is (`None` for the account window).
    fn feature(self) -> Option<(usize, &'static str)> {
        match self {
            Window::Design => Some((0, "design")),
            Window::Redraw => Some((1, "redraw")),
            Window::Ask => Some((2, "ask")),
            Window::Check => Some((3, "check")),
            Window::Account | Window::Publish => None,
        }
    }
}

impl AiState {
    /// The way requests are paid for right now.
    fn auth(&self) -> Result<Auth, AiError> {
        if self.use_key {
            return Ok(Auth::Key(self.key.clone()));
        }
        match &self.token {
            Some(token) => Ok(Auth::Session { server: self.server.clone(), token: token.clone() }),
            None => Err(AiError::SignIn),
        }
    }

    pub(crate) fn signed_in(&self) -> bool {
        self.token.is_some()
    }

    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

impl LaserFoundryApp {
    /// Opens one of the AI windows (loading what was kept the first time).
    pub(crate) fn open_ai(&mut self, window: Window) {
        self.load_ai();
        if !self.ai_feature_on(window) {
            return;
        }
        if self.ai.size_mm <= 0.0 {
            self.ai.size_mm = 100.0;
        }
        if self.ai.window != Some(window) {
            self.ai.note = None; // what the last window said is not about this one
        }
        self.ai.window = Some(window);
    }

    /// Whether the feature behind `window` is switched on.
    pub(crate) fn ai_feature_on(&self, window: Window) -> bool {
        window.feature().is_none_or(|(i, _)| !self.ai.off[i])
    }

    pub(crate) fn load_ai(&mut self) {
        if std::mem::replace(&mut self.ai.loaded, true) {
            return;
        }
        let kept: Option<Value> = self.platform.secret_get(SECRET_KEY).and_then(|t| serde_json::from_str(&t).ok());
        let text = |k: &str| kept.as_ref().and_then(|v| v.get(k)).and_then(Value::as_str).map(str::to_string);
        self.ai.server = text("server").unwrap_or_else(|| cloud::built_in_server().to_string());
        self.ai.email = text("email").unwrap_or_default();
        self.ai.key = text("key").unwrap_or_default();
        self.ai.use_key = kept.as_ref().and_then(|v| v.get("use_key")).and_then(Value::as_bool).unwrap_or(false);
        self.ai.token = text("token").filter(|t| t.starts_with(cloud::TOKEN_PREFIX));
        self.ai.consented = kept.as_ref().and_then(|v| v.get("consented")).and_then(Value::as_bool).unwrap_or(false);
        let off: Vec<&str> = kept
            .as_ref()
            .and_then(|v| v.get("off"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for window in [Window::Design, Window::Redraw, Window::Ask, Window::Check] {
            if let Some((i, name)) = window.feature() {
                self.ai.off[i] = off.contains(&name);
            }
        }
    }

    fn save_ai(&self) {
        let off: Vec<&str> = [Window::Design, Window::Redraw, Window::Ask, Window::Check]
            .into_iter()
            .filter_map(Window::feature)
            .filter(|(i, _)| self.ai.off[*i])
            .map(|(_, name)| name)
            .collect();
        let value = json!({ "server": self.ai.server, "email": self.ai.email, "key": self.ai.key, "use_key": self.ai.use_key, "token": self.ai.token, "consented": self.ai.consented, "off": off });
        self.platform.secret_set(SECRET_KEY, &value.to_string());
    }

    pub(crate) fn ai_error(&mut self, e: &AiError) {
        if matches!(e, AiError::SignIn) && self.ai.token.is_some() && !self.ai.use_key {
            // the session ended: forget it so the window asks for a sign-in
            self.ai.token = None;
            self.ai.account = None;
            self.save_ai();
        }
        self.ai.note = Some((i18n::tr_message(&e.to_string()), true));
    }

    /// Sends `request` in the background, to be read by [`poll_ai`](Self::poll_ai) as `what`.
    pub(crate) fn send_ai(&mut self, what: Pending, request: Result<HttpRequest, AiError>) {
        match request {
            Err(e) => self.ai_error(&e),
            Ok(request) if !self.ai.consented && matches!(what, Pending::Art | Pending::Ask | Pending::Check) => {
                // The first time, say what is sent and where before sending it.
                self.ai.awaiting_consent = Some((what, request));
            }
            Ok(request) => self.dispatch(what, request),
        }
    }

    /// Hands `request` to the host.
    fn dispatch(&mut self, what: Pending, request: HttpRequest) {
        let logging = self.log_entry(what, &request);
        if self.platform.start_http(request) {
            self.ai.logging = logging;
            self.ai.pending = Some(what);
            if what != Pending::Account {
                self.ai.note = None; // a background refresh of the credits keeps what the window just said
            }
        } else {
            self.ai.note = Some((tr("Another request is still running: wait for it to finish."), true));
        }
    }

    /// Reads the answer to the request in flight, once it has come. Call every frame.
    pub(crate) fn poll_ai(&mut self, ctx: &egui::Context) {
        let Some(what) = self.ai.pending else { return };
        let Some(result) = self.platform.take_http() else {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        };
        self.ai.pending = None;
        self.write_log(&result);
        match result {
            Err(e) => self.ai_error(&AiError::Network(e)),
            Ok((status, body)) => self.ai_answer(what, status, &body),
        }
    }

    /// What the log says about a request: the feature, where it went, and the body with pictures left out. Only
    /// the AI requests are logged (not signing in, and never a token or key: headers are not kept).
    fn log_entry(&self, what: Pending, request: &HttpRequest) -> Option<LogEntry> {
        let feature = match what {
            Pending::Art => "art",
            Pending::Ask => "ask",
            Pending::Check => "check",
            _ => return None,
        };
        self.platform.ai_log_file()?;
        let mut body: Value = serde_json::from_str(&request.body).unwrap_or(Value::Null);
        redact_pictures(&mut body);
        let destination = if self.ai.use_key { laserfoundry_ai::API_URL.to_string() } else { request.url.clone() };
        Some(LogEntry { feature, destination, request: body })
    }

    /// Appends the finished request and its answer (or why there was none) to the log of what was sent.
    fn write_log(&mut self, result: &Result<(u16, String), String>) {
        let (Some(entry), Some(path)) = (self.ai.logging.take(), self.platform.ai_log_file()) else { return };
        let answer = match result {
            Ok((status, body)) => json!({ "status": status, "body": body.chars().take(MAX_LOGGED_ANSWER).collect::<String>() }),
            Err(e) => json!({ "error": e }),
        };
        let at = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let line = json!({ "time": at, "feature": entry.feature, "to": entry.destination, "request": entry.request, "answer": answer });
        if let Err(e) = append_log(&path, &line.to_string()) {
            log::warn!("could not write the AI log: {e}");
        }
    }

    fn ai_answer(&mut self, what: Pending, status: u16, body: &str) {
        let session = !self.ai.use_key;
        match what {
            Pending::SignIn | Pending::SignUp => match cloud::parse(status, body) {
                Ok(v) => match cloud::session(&v) {
                    Some((token, email)) => {
                        self.ai.token = Some(token);
                        if !email.is_empty() {
                            self.ai.email = email;
                        }
                        self.ai.password.clear();
                        self.ai.use_key = false;
                        self.save_ai();
                        self.ai.note = Some((tr("Signed in."), false));
                        self.refresh_account();
                    }
                    None => self.ai_error(&AiError::NoAnswer),
                },
                Err(e) => self.ai_error(&e),
            },
            Pending::SignOut => {
                self.ai.token = None;
                self.ai.account = None;
                self.save_ai();
                self.ai.note = Some((tr("Signed out."), false));
            }
            Pending::Account => match cloud::parse(status, body) {
                Ok(v) => self.ai.account = Some(cloud::read_account(&v)),
                Err(e) => self.ai_error(&e),
            },
            Pending::Checkout => match cloud::parse(status, body) {
                Ok(v) => match cloud::checkout_url(&v) {
                    Some(url) => {
                        self.pending_url = Some(url);
                        self.ai.note =
                            Some((tr("The payment page is opening in your browser. Come back and press Refresh when you have paid."), false));
                    }
                    None => self.ai_error(&AiError::NoAnswer),
                },
                Err(e) => self.ai_error(&e),
            },
            Pending::Art => match laserfoundry_ai::art::parse_reply(status, body, session) {
                Ok(art) => {
                    let size = self.ai.size_mm;
                    if let Some(placed) = self.run("ai.place_art", json!({ "svg": art.svg, "size_mm": size })) {
                        let (w, h) = (placed["width_mm"].as_f64().unwrap_or(0.0), placed["height_mm"].as_f64().unwrap_or(0.0));
                        self.ai.note =
                            Some((trf!("Placed the artwork ({w} × {h} mm). Undo removes it.", w = format!("{w:.0}"), h = format!("{h:.0}")), false));
                        self.refresh_account();
                    }
                }
                Err(e) => self.ai_error(&e),
            },
            Pending::Ask => match laserfoundry_ai::ask::parse_reply(status, body, session) {
                Ok(answer) => {
                    self.ai.turns.push(Turn { from_person: false, text: answer });
                    self.refresh_account();
                }
                Err(e) => {
                    self.ai.turns.pop(); // the question that was not answered goes back to the box
                    self.ai_error(&e);
                }
            },
            Pending::Check => match laserfoundry_ai::check::parse_reply(status, body, session) {
                Ok(check) => {
                    self.ai.check = Some(check);
                    self.refresh_account();
                }
                Err(e) => self.ai_error(&e),
            },
            Pending::Publish => self.publish_answered(status, body),
        }
    }

    /// Asks the service for the account's credits (a no-op with the person's own key).
    fn refresh_account(&mut self) {
        if self.ai.use_key || self.ai.pending.is_some() {
            return;
        }
        if let Some(token) = self.ai.token.clone() {
            let request = cloud::account(&self.ai.server, &token);
            self.send_ai(Pending::Account, request);
        }
    }

    /// Describes the open document for the model.
    fn ai_context(&mut self) -> String {
        self.run("ai.context", Value::Null).and_then(|v| v["text"].as_str().map(str::to_string)).unwrap_or_default()
    }

    /// Asks for a picture: a file dialog where there is one, else the browser's file chooser (the picture
    /// arrives later, in [`ai_picture_arrived`](Self::ai_picture_arrived)).
    fn ask_for_picture(&mut self, target: PictureFor) {
        if self.platform.browser_files() {
            self.ai.picture_for = Some(target);
            self.begin_upload(crate::UploadKind::AiPicture);
            return;
        }
        let Some(path) = self.pick_open("Choose a picture", &["png", "jpg", "jpeg", "webp", "gif", "bmp"]) else { return };
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if let Some(v) = self.run("ai.prepare_picture", json!({ "path": path })) {
            self.accept_picture(target, name, &v);
        }
    }

    /// A picture the browser's file chooser returned (as base64): prepared and kept for the window that asked.
    /// Returns whether it was used.
    pub(crate) fn ai_picture_arrived(&mut self, name: &str, data_base64: &str) -> bool {
        let Some(target) = self.ai.picture_for.take() else { return false };
        match self.run("ai.prepare_picture", json!({ "data_base64": data_base64 })) {
            Some(v) => {
                self.accept_picture(target, name.chars().take(80).collect(), &v);
                true
            }
            None => false,
        }
    }

    fn accept_picture(&mut self, target: PictureFor, name: String, prepared: &Value) {
        let Some(data) = prepared["data_base64"].as_str() else { return };
        let picture = Picture { media_type: "image/jpeg", data_base64: data.to_string() };
        match target {
            PictureFor::Redraw => self.ai.picture = Some((name, picture)),
            PictureFor::Check => self.ai.photos.push((name, picture)),
        }
    }

    fn start_design(&mut self, redraw: bool) {
        let mut request = ArtRequest::new(&self.ai.prompt, self.ai.size_mm, self.ai.use_for);
        if redraw {
            match &self.ai.picture {
                Some((_, p)) => request = request.with_image(p.clone()),
                None => {
                    self.ai.note = Some((tr("Choose a picture first."), true));
                    return;
                }
            }
        }
        let http = self.ai.auth().and_then(|auth| request.http(&auth));
        self.send_ai(Pending::Art, http);
    }

    fn start_ask(&mut self) {
        let turn = match laserfoundry_ai::ask::question(&self.ai.question) {
            Ok(t) => t,
            Err(e) => return self.ai_error(&e),
        };
        self.ai.turns.push(turn);
        let context = self.ai_context();
        let http = self.ai.auth().and_then(|auth| laserfoundry_ai::ask::request(&auth, &context, &self.ai.turns));
        if http.is_ok() {
            self.ai.question.clear();
        }
        let sent = self.ai.pending;
        self.send_ai(Pending::Ask, http);
        if self.ai.pending == sent {
            self.ai.turns.pop(); // nothing was sent
        }
    }

    fn start_check(&mut self) {
        let settings = self.ai_context();
        let request = CheckRequest { photos: self.ai.photos.iter().map(|(_, p)| p.clone()).collect(), notes: self.ai.check_notes.clone(), settings };
        let http = self.ai.auth().and_then(|auth| request.http(&auth));
        self.ai.check = None;
        self.send_ai(Pending::Check, http);
    }

    /// Applies one of the model's suggestions to its layer: an ordinary command, one undo step.
    pub(crate) fn apply_suggestion(&mut self, index: usize) {
        let Some(s) = self.ai.check.as_ref().and_then(|c| c.suggestions.get(index)).cloned() else { return };
        let mut params = json!({ "id": s.layer });
        if let Some(v) = s.speed {
            params["speed"] = json!(v);
        }
        if let Some(v) = s.max_power {
            params["max_power"] = json!(v);
        }
        if let Some(v) = s.passes {
            params["passes"] = json!(v);
        }
        if self.run("layer.set", params).is_some() {
            self.say(trf!("Changed layer {layer}'s settings", layer = s.layer));
        }
    }

    /// The person's answer to the first-use question: send the waiting request, or drop it.
    fn answer_consent(&mut self, agree: bool, always: bool) {
        let waiting = self.ai.awaiting_consent.take();
        if agree && always {
            self.ai.consented = true;
            self.save_ai();
        }
        if let (true, Some((what, request))) = (agree, waiting) {
            self.dispatch(what, request);
        }
    }

    /// Asks, once, whether the request may go to the AI service.
    fn consent_dialog(&mut self, ctx: &egui::Context) {
        let Some((_, request)) = &self.ai.awaiting_consent else { return };
        let destination = if self.ai.use_key { tr("Anthropic, with your API key") } else { request.url.clone() };
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("ai_consent")).show(ctx, |ui| {
            ui.heading(tr("Send this to the AI service?"));
            ui.label(trf!("It goes to: {destination}", destination = destination));
            ui.label(tr("What is sent: your description or question, any picture you chose, and a plain-text summary of your machine and the layers in use (never your artwork). The AI service and the model provider see it."));
            ui.label(tr("Do not send anything private. Use the AI lawfully: nothing Anthropic's usage policy forbids (anthropic.com/legal/aup)."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Send")).clicked() {
                    choice = Some((true, false));
                }
                if ui.button(tr("Send, and do not ask again")).clicked() {
                    choice = Some((true, true));
                }
                if ui.button(tr("Cancel")).clicked() {
                    choice = Some((false, false));
                }
            });
        });
        if let Some((agree, always)) = choice {
            self.answer_consent(agree, always);
        }
    }

    /// Draws whichever AI window is open.
    pub(crate) fn ai_windows(&mut self, ctx: &egui::Context) {
        self.consent_dialog(ctx);
        let Some(window) = self.ai.window else { return };
        let mut open = true;
        let title = match window {
            Window::Account => tr("Foundry Account"),
            Window::Design => tr("Design from Prompt"),
            Window::Redraw => tr("Redraw Image"),
            Window::Ask => tr("Ask About Lasering"),
            Window::Check => tr("Check My Cut"),
            Window::Publish => tr("Publish to MakerFoundry"),
        };
        egui::Window::new(title).open(&mut open).collapsible(false).default_width(460.0).default_pos(egui::pos2(260.0, 70.0)).show(ctx, |ui| {
            match window {
                Window::Account => self.account_window(ui),
                Window::Design => self.design_window(ui, false),
                Window::Redraw => self.design_window(ui, true),
                Window::Ask => self.ask_window(ui),
                Window::Check => self.check_window(ui),
                Window::Publish => self.publish_window(ui),
            }
        });
        if !open {
            self.ai.window = None;
        }
    }

    fn payer_row(&mut self, ui: &mut egui::Ui) {
        if self.ai.use_key {
            ui.weak(tr("Paying with your own Anthropic key."));
        } else if let Some(a) = &self.ai.account {
            ui.horizontal(|ui| {
                ui.label(trf!("Foundry credits: {credits}", credits = a.credits));
                if ui.small_button(tr("Foundry Account…")).clicked() {
                    self.ai.window = Some(Window::Account);
                }
            });
        } else if !self.ai.signed_in() {
            ui.horizontal(|ui| {
                ui.colored_label(self.tokens.error, tr("Not signed in."));
                if ui.small_button(tr("Foundry Account…")).clicked() {
                    self.ai.window = Some(Window::Account);
                }
            });
        }
    }

    pub(crate) fn note_row(&self, ui: &mut egui::Ui) {
        if let Some((text, error)) = &self.ai.note {
            ui.colored_label(if *error { self.tokens.error } else { ui.visuals().text_color() }, text);
        }
        if self.ai.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Waiting for the answer…"));
            });
        }
    }

    fn account_window(&mut self, ui: &mut egui::Ui) {
        let busy = self.ai.busy();
        ui.checkbox(&mut self.ai.use_key, tr("Use my own Anthropic API key instead of an account"));
        if self.ai.use_key {
            ui.horizontal(|ui| {
                ui.label(tr("API key"));
                ui.add(egui::TextEdit::singleline(&mut self.ai.key).password(true).desired_width(300.0));
            });
            ui.weak(tr("Requests go straight to Anthropic and are billed to your key. The key is kept on this computer."));
            if ui.button(tr("Save")).clicked() {
                self.save_ai();
                self.ai.note = Some((tr("Saved."), false));
            }
        } else {
            ui.horizontal(|ui| {
                ui.label(tr("Server"));
                ui.add(egui::TextEdit::singleline(&mut self.ai.server).desired_width(320.0).hint_text("https://…"));
            });
            if self.ai.signed_in() {
                ui.label(trf!("Signed in as {email}", email = self.ai.email));
                if let Some(a) = self.ai.account.clone() {
                    ui.label(trf!("Foundry credits: {credits}", credits = a.credits));
                    if a.payments {
                        for pack in &a.packs {
                            let label = trf!("Buy {credits} Foundry credits for {price}", credits = pack.credits, price = cloud::dollars(pack.cents));
                            if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                                let request = cloud::checkout(&self.ai.server, self.ai.token.as_deref().unwrap_or_default(), &pack.id);
                                self.send_ai(Pending::Checkout, request);
                            }
                        }
                    } else {
                        ui.weak(tr("This service is not taking payments yet."));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new(tr("Refresh"))).clicked() {
                        self.refresh_account();
                    }
                    if ui.add_enabled(!busy, egui::Button::new(tr("Sign out"))).clicked() {
                        let request = cloud::sign_out(&self.ai.server, self.ai.token.as_deref().unwrap_or_default());
                        self.send_ai(Pending::SignOut, request);
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(tr("Email"));
                    ui.add(egui::TextEdit::singleline(&mut self.ai.email).desired_width(260.0));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Password"));
                    ui.add(egui::TextEdit::singleline(&mut self.ai.password).password(true).desired_width(260.0));
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new(tr("Sign in"))).clicked() {
                        let request = cloud::sign_in(&self.ai.server, &self.ai.email, &self.ai.password);
                        self.save_ai();
                        self.send_ai(Pending::SignIn, request);
                    }
                    if ui.add_enabled(!busy, egui::Button::new(tr("Create account"))).clicked() {
                        let request = cloud::sign_up(&self.ai.server, &self.ai.email, &self.ai.password);
                        self.save_ai();
                        self.send_ai(Pending::SignUp, request);
                    }
                });
            }
        }
        ui.separator();
        ui.label(tr("Features"));
        let mut changed = false;
        for (window, label) in [
            (Window::Design, tr("Design from Prompt")),
            (Window::Redraw, tr("Redraw Image")),
            (Window::Ask, tr("Ask About Lasering")),
            (Window::Check, tr("Check My Cut")),
        ] {
            if let Some((i, _)) = window.feature() {
                let mut on = !self.ai.off[i];
                if ui.checkbox(&mut on, label).changed() {
                    self.ai.off[i] = !on;
                    changed = true;
                }
            }
        }
        if changed {
            self.save_ai();
        }
        ui.weak(tr("What you ask is sent to the AI service. Nothing is sent until you press a button."));
        if let Some(path) = self.platform.ai_log_file() {
            ui.weak(trf!("A log of every request and answer is kept in {path}, so you can check what was sent.", path = path.display()));
        }
        self.note_row(ui);
    }

    fn design_window(&mut self, ui: &mut egui::Ui, redraw: bool) {
        let busy = self.ai.busy();
        if redraw {
            ui.horizontal(|ui| {
                if ui.button(tr("Choose picture…")).clicked() {
                    self.ask_for_picture(PictureFor::Redraw);
                }
                match &self.ai.picture {
                    Some((name, _)) => ui.label(name),
                    None => ui.weak(tr("No picture chosen")),
                };
            });
            ui.weak(tr("Use pictures you own or may use: the AI will not copy someone else's logo or character."));
            ui.label(tr("What to change (optional)"));
        } else {
            ui.label(tr("Describe the design"));
        }
        ui.add(egui::TextEdit::multiline(&mut self.ai.prompt).desired_rows(3).desired_width(f32::INFINITY));
        ui.horizontal(|ui| {
            ui.label(tr("Size (longest side)"));
            ui.add(egui::DragValue::new(&mut self.ai.size_mm).range(10.0..=1000.0).suffix(" mm"));
            egui::ComboBox::from_id_salt("ai_use").selected_text(use_label(self.ai.use_for)).show_ui(ui, |ui| {
                for u in [Use::CutAndEngrave, Use::Engrave, Use::Cut] {
                    ui.selectable_value(&mut self.ai.use_for, u, use_label(u));
                }
            });
        });
        ui.weak(tr("Red outlines are cut, blue lines scored, black shapes engraved. Undo removes what is placed."));
        ui.weak(tr("The AI can get it wrong: look over the artwork before you cut anything."));
        self.payer_row(ui);
        if ui.add_enabled(!busy, egui::Button::new(tr("Draw"))).clicked() {
            self.start_design(redraw);
        }
        self.note_row(ui);
    }

    fn ask_window(&mut self, ui: &mut egui::Ui) {
        let busy = self.ai.busy();
        egui::ScrollArea::vertical().max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
            for turn in &self.ai.turns {
                if turn.from_person {
                    ui.strong(&turn.text);
                } else {
                    ui.label(&turn.text);
                }
                ui.add_space(6.0);
            }
        });
        ui.add(egui::TextEdit::multiline(&mut self.ai.question).desired_rows(2).desired_width(f32::INFINITY).hint_text(tr("Ask a question…")));
        ui.weak(tr(
            "The AI is told about your machine and the layers in use, not your artwork. Answers can be wrong: read them, test on a scrap, and never leave a running laser unattended.",
        ));
        self.payer_row(ui);
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new(tr("Ask"))).clicked() {
                self.start_ask();
            }
            if ui.add_enabled(!busy && !self.ai.turns.is_empty(), egui::Button::new(tr("New conversation"))).clicked() {
                self.ai.turns.clear();
            }
        });
        self.note_row(ui);
    }

    fn check_window(&mut self, ui: &mut egui::Ui) {
        let busy = self.ai.busy();
        ui.horizontal(|ui| {
            if ui.add_enabled(self.ai.photos.len() < laserfoundry_ai::MAX_IMAGES, egui::Button::new(tr("Add photo…"))).clicked() {
                self.ask_for_picture(PictureFor::Check);
            }
            if !self.ai.photos.is_empty() && ui.button(tr("Remove photos")).clicked() {
                self.ai.photos.clear();
            }
        });
        for (name, _) in &self.ai.photos {
            ui.label(name);
        }
        ui.label(tr("Material and what you see (optional)"));
        ui.add(egui::TextEdit::multiline(&mut self.ai.check_notes).desired_rows(2).desired_width(f32::INFINITY));
        ui.weak(tr("The photos, your notes and your layers' settings are sent to the AI service. Suggestions can be wrong: try them on a scrap."));
        self.payer_row(ui);
        if ui.add_enabled(!busy, egui::Button::new(tr("Check"))).clicked() {
            self.start_check();
        }
        self.note_row(ui);
        if let Some(check) = self.ai.check.clone() {
            ui.separator();
            ui.label(&check.summary);
            for (i, s) in check.suggestions.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    let mut parts = Vec::new();
                    if let Some(v) = s.speed {
                        parts.push(trf!("speed {v} mm/min", v = v));
                    }
                    if let Some(v) = s.max_power {
                        parts.push(trf!("power {v} %", v = v));
                    }
                    if let Some(v) = s.passes {
                        parts.push(trf!("{v} passes", v = v));
                    }
                    ui.label(trf!("Layer {layer}: {changes}", layer = s.layer, changes = parts.join(", ")));
                    if ui.small_button(tr("Apply")).clicked() {
                        self.apply_suggestion(i);
                    }
                });
                if !s.reason.is_empty() {
                    ui.weak(&s.reason);
                }
            }
        }
    }
}

/// The most characters of an answer kept in the log.
const MAX_LOGGED_ANSWER: usize = 100_000;
/// The log is moved aside (keeping one older copy) when it grows past this.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Replaces the data of every picture in a request body with its size, so the log shows that one was sent
/// without holding it.
fn redact_pictures(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if let Some(Value::String(data)) = map.get_mut("data") {
                *data = format!("<picture, {} base64 characters>", data.len());
            }
            map.values_mut().for_each(redact_pictures);
        }
        Value::Array(list) => list.iter_mut().for_each(redact_pictures),
        _ => {}
    }
}

/// Adds a line to the log at `path`, readable by its owner only; an old log is moved aside first.
fn append_log(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("jsonl.1"));
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    writeln!(options.open(path)?, "{line}")
}

fn use_label(u: Use) -> String {
    match u {
        Use::CutAndEngrave => tr("Cut and engrave"),
        Use::Engrave => tr("Engrave only"),
        Use::Cut => tr("Cut only"),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::path::PathBuf;
    use std::rc::Rc;

    use laserfoundry_device::Transport;
    use laserfoundry_engine::Session;

    use super::*;
    use crate::Platform;

    #[derive(Default)]
    struct Shared {
        /// Requests the app sent, oldest first.
        sent: Vec<HttpRequest>,
        /// Answers to give, oldest first.
        answers: VecDeque<Result<(u16, String), String>>,
        in_flight: bool,
        secrets: HashMap<String, String>,
        picture: Option<PathBuf>,
        /// Acts like a browser host: files arrive as uploads.
        browser: bool,
        upload: Option<crate::Upload>,
        requested: Vec<Vec<String>>,
        log: Option<PathBuf>,
    }

    struct Fake(Rc<RefCell<Shared>>);

    impl Platform for Fake {
        fn pick_open(&self, _: &str, _: &[&str]) -> Option<PathBuf> {
            self.0.borrow().picture.clone()
        }

        fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<PathBuf> {
            None
        }

        fn serial_ports(&self) -> Vec<String> {
            Vec::new()
        }

        fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
            Err("no serial".into())
        }

        fn http_available(&self) -> bool {
            true
        }

        fn browser_files(&self) -> bool {
            self.0.borrow().browser
        }

        fn ai_log_file(&self) -> Option<PathBuf> {
            self.0.borrow().log.clone()
        }

        fn request_upload(&self, extensions: &[&str]) {
            self.0.borrow_mut().requested.push(extensions.iter().map(|e| (*e).to_string()).collect());
        }

        fn take_upload(&self) -> Option<crate::Upload> {
            self.0.borrow_mut().upload.take()
        }

        fn start_http(&self, request: HttpRequest) -> bool {
            let mut s = self.0.borrow_mut();
            if s.in_flight {
                return false;
            }
            s.in_flight = true;
            s.sent.push(request);
            true
        }

        fn take_http(&self) -> Option<Result<(u16, String), String>> {
            let mut s = self.0.borrow_mut();
            let answer = s.answers.pop_front()?;
            s.in_flight = false;
            Some(answer)
        }

        fn secret_get(&self, key: &str) -> Option<String> {
            self.0.borrow().secrets.get(key).cloned()
        }

        fn secret_set(&self, key: &str, value: &str) {
            self.0.borrow_mut().secrets.insert(key.to_string(), value.to_string());
        }
    }

    /// An app whose person has already agreed to what is sent (the first-use question has its own test).
    fn app() -> (LaserFoundryApp, Rc<RefCell<Shared>>) {
        let shared = Rc::new(RefCell::new(Shared::default()));
        shared.borrow_mut().secrets.insert(SECRET_KEY.into(), json!({ "consented": true }).to_string());
        (LaserFoundryApp::new(Session::new(), Box::new(Fake(Rc::clone(&shared)))), shared)
    }

    /// Gives the answer to the request in flight and lets the app read it.
    fn answer(app: &mut LaserFoundryApp, shared: &Rc<RefCell<Shared>>, status: u16, body: &str) {
        shared.borrow_mut().answers.push_back(Ok((status, body.to_string())));
        app.poll_ai(&egui::Context::default());
    }

    /// Answers the credits refresh that follows a paid request, if one was sent.
    fn settle(app: &mut LaserFoundryApp, shared: &Rc<RefCell<Shared>>) {
        if app.ai.pending == Some(Pending::Account) {
            answer(app, shared, 200, r#"{"email":"me@example.com","credits":90,"payments":true,"packs":[]}"#);
        }
    }

    fn model_reply(text: &str) -> String {
        json!({"type": "message", "model": "m", "stop_reason": "end_turn", "content": [{"type": "text", "text": text}]}).to_string()
    }

    fn sign_in(app: &mut LaserFoundryApp, shared: &Rc<RefCell<Shared>>) {
        app.open_ai(Window::Account);
        app.ai.server = "https://c.example".into();
        app.ai.email = "me@example.com".into();
        app.ai.password = "secret words".into();
        let request = cloud::sign_in(&app.ai.server, &app.ai.email, &app.ai.password);
        app.send_ai(Pending::SignIn, request);
        answer(app, shared, 200, r#"{"token":"sfs_tok","account":{"email":"me@example.com","credits":100}}"#);
        answer(app, shared, 200, r#"{"email":"me@example.com","credits":100,"payments":true,"packs":[{"id":"small","credits":1000,"cents":1000}]}"#);
    }

    #[test]
    fn signing_in_keeps_the_session_privately_and_reads_the_credits() {
        let (mut app, shared) = app();
        sign_in(&mut app, &shared);
        assert_eq!(app.ai.token.as_deref(), Some("sfs_tok"));
        assert!(app.ai.password.is_empty(), "the password is not kept");
        assert_eq!(app.ai.account.as_ref().map(|a| a.credits), Some(100));
        let kept = shared.borrow().secrets[SECRET_KEY].clone();
        assert!(kept.contains("sfs_tok") && !kept.contains("secret words"), "{kept}");
        let sent = &shared.borrow().sent;
        assert_eq!(sent[0].url, "https://c.example/v1/signin");
        assert_eq!(sent[1].url, "https://c.example/v1/account");
        // a new start reads the session back
        let (mut again, _) = app_with(&shared);
        again.open_ai(Window::Ask);
        assert_eq!(again.ai.token.as_deref(), Some("sfs_tok"));
        assert_eq!(again.ai.server, "https://c.example");
    }

    fn app_with(shared: &Rc<RefCell<Shared>>) -> (LaserFoundryApp, ()) {
        (LaserFoundryApp::new(Session::new(), Box::new(Fake(Rc::clone(shared)))), ())
    }

    #[test]
    fn design_from_prompt_places_the_artwork_and_a_refusal_places_nothing() {
        let (mut app, shared) = app();
        sign_in(&mut app, &shared);
        app.ai.prompt = "a small fox".into();
        app.ai.size_mm = 60.0;
        app.start_design(false);
        assert_eq!(app.ai.pending, Some(Pending::Art));
        let sent = shared.borrow().sent.last().cloned().unwrap();
        assert_eq!(sent.url, "https://c.example/v1/messages");
        assert!(sent.headers.contains(&("authorization", "Bearer sfs_tok".to_string())));
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="30mm" height="20mm" viewBox="0 0 30 20"><rect width="30" height="20" fill="none" stroke="#ff0000"/></svg>"##;
        answer(&mut app, &shared, 200, &model_reply(svg));
        assert!(matches!(&app.ai.note, Some((_, false))), "{:?}", app.ai.note);
        settle(&mut app, &shared);
        assert_eq!(app.ai.account.as_ref().map(|a| a.credits), Some(90), "the credits are read again after a paid request");
        assert_eq!(app.session.doc.shapes.len(), 1);
        let b = app.session.doc.bounds_of(&app.session.selection).unwrap();
        assert!((b.width() - 60.0).abs() < 0.5, "sized as asked: {b:?}");
        assert!(matches!(&app.ai.note, Some((_, false))), "{:?}", app.ai.note);
        // refused: nothing more is placed, the reason is shown
        app.start_design(false);
        answer(&mut app, &shared, 200, &model_reply("DECLINED: that is a brand's logo"));
        assert_eq!(app.session.doc.shapes.len(), 1);
        assert!(matches!(&app.ai.note, Some((m, true)) if m.contains("logo")), "{:?}", app.ai.note);
        // junk artwork is reported, not fatal
        app.start_design(false);
        answer(&mut app, &shared, 200, &model_reply("<svg><nonsense"));
        assert_eq!(app.session.doc.shapes.len(), 1);
    }

    #[test]
    fn the_first_request_waits_for_the_persons_agreement_and_says_where_it_goes() {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let mut app = LaserFoundryApp::new(Session::new(), Box::new(Fake(Rc::clone(&shared))));
        app.open_ai(Window::Ask);
        app.ai.use_key = true;
        app.ai.key = "sk-ant-x".into();
        app.ai.question = "hello".into();
        app.start_ask();
        assert!(app.ai.awaiting_consent.is_some() && shared.borrow().sent.is_empty(), "nothing is sent before the person agrees");
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| app.ai_windows(ui.ctx())).drop_without_applying_deltas();
        // cancel: nothing is sent, and it is not remembered
        app.answer_consent(false, true);
        assert!(shared.borrow().sent.is_empty() && !app.ai.consented && app.ai.awaiting_consent.is_none());
        // send once: it goes, and the question is asked again next time
        app.ai.question = "hello".into();
        app.start_ask();
        app.answer_consent(true, false);
        assert_eq!(shared.borrow().sent.len(), 1);
        assert!(!app.ai.consented);
        answer(&mut app, &shared, 200, &model_reply("hi"));
        app.ai.question = "again".into();
        app.start_ask();
        assert!(app.ai.awaiting_consent.is_some(), "asked again");
        // send and do not ask again: kept with the settings
        app.answer_consent(true, true);
        assert!(app.ai.consented && shared.borrow().secrets[SECRET_KEY].contains("\"consented\":true"));
        answer(&mut app, &shared, 200, &model_reply("hi"));
        app.ai.question = "third".into();
        app.start_ask();
        assert!(app.ai.awaiting_consent.is_none() && shared.borrow().sent.len() == 3);
        // signing in is not an AI request and never asks
        let mut app2 = LaserFoundryApp::new(Session::new(), Box::new(Fake(Rc::clone(&shared))));
        app2.ai.consented = false;
        let request = cloud::sign_in("https://c.example", "a@b.c", "pw pw pw pw");
        app2.send_ai(Pending::SignIn, request);
        assert!(app2.ai.awaiting_consent.is_none());
    }

    #[test]
    fn requests_that_cannot_be_made_say_why_and_send_nothing() {
        let (mut app, shared) = app();
        app.open_ai(Window::Design);
        app.ai.prompt = "a fox".into();
        app.start_design(false);
        assert!(matches!(&app.ai.note, Some((m, true)) if m.contains("Sign in")), "{:?}", app.ai.note);
        app.ai.prompt.clear();
        app.ai.use_key = true;
        app.ai.key = "sk-ant-x".into();
        app.start_design(false);
        assert!(matches!(&app.ai.note, Some((m, true)) if m.contains("Describe")), "{:?}", app.ai.note);
        app.start_design(true);
        assert!(matches!(&app.ai.note, Some((m, true)) if m.contains("picture")), "{:?}", app.ai.note);
        assert!(shared.borrow().sent.is_empty());
        // one request at a time
        app.ai.prompt = "a fox".into();
        app.start_design(false);
        assert_eq!(shared.borrow().sent[0].url, laserfoundry_ai::API_URL, "the person's own key goes to Anthropic");
        app.start_design(false);
        assert_eq!(shared.borrow().sent.len(), 1);
        assert!(matches!(&app.ai.note, Some((m, true)) if m.contains("still running")), "{:?}", app.ai.note);
    }

    #[test]
    fn an_ended_session_forgets_itself_and_not_enough_credits_is_said() {
        let (mut app, shared) = app();
        sign_in(&mut app, &shared);
        app.ai.question = "hi".into();
        app.start_ask();
        answer(&mut app, &shared, 402, r#"{"error":{"message":"x"}}"#);
        assert!(matches!(&app.ai.note, Some((m, true)) if m.contains("credits")), "{:?}", app.ai.note);
        assert!(app.ai.token.is_some() && app.ai.turns.is_empty(), "an unanswered question is not kept in the conversation");
        app.ai.question = "hi".into();
        app.start_ask();
        answer(&mut app, &shared, 401, "{}");
        assert!(app.ai.token.is_none() && app.ai.account.is_none(), "the session ended");
        assert!(shared.borrow().secrets[SECRET_KEY].contains("\"token\":null"));
    }

    #[test]
    fn asking_sends_the_document_as_context_and_keeps_the_conversation() {
        let (mut app, shared) = app();
        sign_in(&mut app, &shared);
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
        app.ai.question = "  Is 1000 mm/min right for 3 mm ply?  ".into();
        app.start_ask();
        let body: Value = serde_json::from_str(&shared.borrow().sent.last().unwrap().body).unwrap();
        assert!(body["system"].as_str().unwrap().contains("Layer 0"), "the layers in use are told");
        assert_eq!(body["messages"][0]["content"], "Is 1000 mm/min right for 3 mm ply?");
        assert!(app.ai.question.is_empty());
        answer(&mut app, &shared, 200, &model_reply("Try 1200 mm/min at 70 %."));
        settle(&mut app, &shared);
        assert_eq!(app.ai.turns.len(), 2);
        assert_eq!(app.ai.turns[1].text, "Try 1200 mm/min at 70 %.");
        app.ai.question = "and two passes?".into();
        app.start_ask();
        let body: Value = serde_json::from_str(&shared.borrow().sent.last().unwrap().body).unwrap();
        assert_eq!(body["messages"].as_array().unwrap().len(), 3, "the earlier turns go along");
    }

    #[test]
    fn check_my_cut_suggestions_apply_as_one_undo_step_each() {
        let (mut app, shared) = app();
        sign_in(&mut app, &shared);
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
        // a photo from a real file
        let dir = std::env::temp_dir().join(format!("lf-ai-photo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cut.png");
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(40, 30, image::Rgb([120, 80, 40]))).save(&file).unwrap();
        shared.borrow_mut().picture = Some(file);
        app.ask_for_picture(PictureFor::Check);
        assert_eq!(app.ai.photos.len(), 1);
        assert_eq!(app.ai.photos[0].0, "cut.png");
        app.ai.check_notes = "3 mm birch, charred".into();
        app.start_check();
        let body: Value = serde_json::from_str(&shared.borrow().sent.last().unwrap().body).unwrap();
        assert_eq!(body["messages"][0]["content"][0]["type"], "image");
        let reply = r#"{"summary":"Too hot.","suggestions":[{"layer":0,"speed":1500,"max_power":50,"passes":2,"reason":"less heat"},{"layer":99,"speed":10}]}"#;
        answer(&mut app, &shared, 200, &model_reply(reply));
        settle(&mut app, &shared);
        let check = app.ai.check.clone().unwrap();
        assert_eq!((check.summary.as_str(), check.suggestions.len()), ("Too hot.", 1), "the layer that does not exist is dropped");
        let before = app.session.doc.layer(0).unwrap().clone();
        app.apply_suggestion(0);
        let l = app.session.doc.layer(0).unwrap();
        assert_eq!((l.speed, l.max_power, l.passes), (1500.0, 50.0, 2));
        app.session.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(app.session.doc.layer(0).unwrap().speed, before.speed, "one undo step");
        app.apply_suggestion(7); // no such suggestion: nothing happens
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_ai_request_and_answer_is_logged_without_secrets_or_pictures() {
        let (mut app, shared) = app();
        let dir = std::env::temp_dir().join(format!("lf-ai-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = dir.join("ai-log.jsonl");
        shared.borrow_mut().log = Some(log.clone());
        sign_in(&mut app, &shared);
        assert!(!log.exists(), "signing in is not an AI request");
        app.ai.photos.push(("p.jpg".into(), Picture { media_type: "image/jpeg", data_base64: "QUJD".repeat(1000) }));
        app.ai.check_notes = "3 mm ply".into();
        app.start_check();
        answer(&mut app, &shared, 200, &model_reply(r#"{"summary":"ok","suggestions":[]}"#));
        settle(&mut app, &shared);
        app.ai.question = "hello there".into();
        app.start_ask();
        answer(&mut app, &shared, 500, "oops");
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!((lines[0]["feature"].as_str(), lines[1]["feature"].as_str()), (Some("check"), Some("ask")));
        assert_eq!(lines[0]["to"], "https://c.example/v1/messages");
        assert!(
            lines[0]["request"].to_string().contains("3 mm ply") && lines[0]["request"].to_string().contains("<picture, 4000 base64 characters>")
        );
        assert!(!text.contains("QUJDQUJD"), "the picture itself is not kept");
        assert!(!text.contains("sfs_tok") && !text.contains("secret words"), "no token or password in the log");
        assert_eq!(lines[0]["answer"]["status"], 200);
        assert_eq!(lines[1]["answer"]["status"], 500);
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&log).unwrap().permissions()) & 0o077, 0, "owner only");
        // without a place to keep a log, nothing is written and nothing breaks
        shared.borrow_mut().log = None;
        app.ai.question = "again".into();
        app.start_ask();
        answer(&mut app, &shared, 200, &model_reply("fine"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_big_log_is_moved_aside() {
        let dir = std::env::temp_dir().join(format!("lf-ai-log-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("ai-log.jsonl");
        std::fs::write(&log, vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();
        append_log(&log, "{\"new\":1}").unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "{\"new\":1}");
        assert!(dir.join("ai-log.jsonl.1").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_feature_can_be_switched_off_and_stays_off() {
        let (mut app, shared) = app();
        app.open_ai(Window::Account);
        assert!(app.ai_feature_on(Window::Ask) && app.ai_feature_on(Window::Account));
        app.ai.off[2] = true; // Ask About Lasering
        app.save_ai();
        app.ai.window = None;
        app.open_ai(Window::Ask);
        assert_eq!(app.ai.window, None, "a switched-off feature does not open");
        assert!(shared.borrow().secrets[SECRET_KEY].contains("\"off\":[\"ask\"]"));
        let (again, _) = app_with(&shared);
        assert!(
            !again.ai_feature_on(Window::Ask) && again.ai_feature_on(Window::Design) && again.ai_feature_on(Window::Account),
            "kept between starts"
        );
    }

    #[test]
    fn a_browser_gets_its_pictures_through_the_file_chooser() {
        let (mut app, shared) = app();
        shared.borrow_mut().browser = true;
        app.open_ai(Window::Redraw);
        app.ask_for_picture(PictureFor::Redraw);
        assert_eq!(shared.borrow().requested.last().unwrap(), &["png", "jpg", "jpeg", "webp", "gif", "bmp"]);
        assert!(app.ai.picture.is_none(), "nothing until the file arrives");
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(20, 10, image::Rgb([9, 9, 9])))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        shared.borrow_mut().upload = Some(crate::Upload { name: "logo.png".into(), bytes: png });
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| app.ui(ui)).drop_without_applying_deltas();
        assert_eq!(app.ai.picture.as_ref().map(|p| p.0.as_str()), Some("logo.png"));
        // a photo for Check My Cut goes to the photos, and junk is refused
        app.ask_for_picture(PictureFor::Check);
        shared.borrow_mut().upload = Some(crate::Upload { name: "bad.png".into(), bytes: b"not a picture".to_vec() });
        ctx.run_ui(egui::RawInput::default(), |ui| app.ui(ui)).drop_without_applying_deltas();
        assert!(app.ai.photos.is_empty());
        assert!(app.ai.picture_for.is_none(), "the question is answered either way");
    }

    #[test]
    fn buying_credits_opens_only_an_https_page_and_the_windows_draw() {
        let (mut app, shared) = app();
        sign_in(&mut app, &shared);
        let request = cloud::checkout(&app.ai.server, "sfs_tok", "small");
        app.send_ai(Pending::Checkout, request);
        answer(&mut app, &shared, 200, r#"{"url":"https://checkout.stripe.com/c/pay/x"}"#);
        assert_eq!(app.pending_url.as_deref(), Some("https://checkout.stripe.com/c/pay/x"));
        app.pending_url = None;
        let request = cloud::checkout(&app.ai.server, "sfs_tok", "small");
        app.send_ai(Pending::Checkout, request);
        answer(&mut app, &shared, 200, r#"{"url":"javascript:alert(1)"}"#);
        assert!(app.pending_url.is_none());
        // every window draws in every state
        let ctx = egui::Context::default();
        for window in [Window::Account, Window::Design, Window::Redraw, Window::Ask, Window::Check] {
            app.ai.window = Some(window);
            ctx.run_ui(egui::RawInput::default(), |ui| app.ai_windows(ui.ctx())).drop_without_applying_deltas();
        }
        app.ai.token = None;
        app.ai.window = Some(Window::Account);
        ctx.run_ui(egui::RawInput::default(), |ui| app.ai_windows(ui.ctx())).drop_without_applying_deltas();
    }
}
