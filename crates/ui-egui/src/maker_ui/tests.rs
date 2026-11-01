#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use laserfoundry_ai::HttpRequest;
use laserfoundry_device::Transport;
use laserfoundry_engine::Session;
use serde_json::{Value, json};

use super::*;
use crate::{Fetched, Platform};

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="20mm" viewBox="0 0 40 20"><rect width="40" height="20" fill="none" stroke="#ff0000"/><circle cx="20" cy="10" r="5" fill="none" stroke="#0000ff"/></svg>"##;
const DXF: &str = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n30\n21\n10\n0\nENDSEC\n0\nEOF\n";
const LINK: &str = "https://makerfoundry.app/api/v1/designs/42/file/rose.svg?t=abc";

mod open_tests {
    use super::*;

    #[test]
    fn a_link_is_checked_fetched_without_credentials_and_opened_like_a_dropped_file() {
        let (mut app, shared) = app_opened_with(LINK);
        poll(&mut app);
        assert_eq!(shared.borrow().fetches, vec![(LINK.to_string(), maker::MAX_OPEN_BYTES)], "exactly the checked address, with the size cap");
        assert!(app.session.doc.shapes.is_empty(), "nothing opens before the answer");
        assert_eq!(status(&app).0, "Opening the design from MakerFoundry…");
        fetched(&shared, 200, SVG.as_bytes(), Some("attachment; filename=\"rose.svg\""));
        poll(&mut app);
        assert_eq!(app.session.doc.shapes.len(), 2, "the rectangle and the circle");
        assert_eq!(status(&app), ("Opened rose.svg".to_string(), false));
        // the link is read once: more frames do not fetch again
        poll(&mut app);
        poll(&mut app);
        assert_eq!(shared.borrow().fetches.len(), 1);
    }

    #[test]
    fn what_is_fetched_is_the_checked_address_not_the_text_that_came_in() {
        let (mut app, shared) = app_opened_with("  https://MakerFoundry.APP/api/v1/designs/1/file/a.svg?t=x#frag  ");
        poll(&mut app);
        assert_eq!(shared.borrow().fetches[0].0, "https://makerfoundry.app/api/v1/designs/1/file/a.svg?t=x");
    }

    #[test]
    fn a_dxf_is_opened_by_its_name_and_the_header_beats_the_address() {
        let (mut app, shared) = app_opened_with("https://www.makerfoundry.app/api/v1/designs/7/file/a.svg");
        poll(&mut app);
        fetched(&shared, 200, DXF.as_bytes(), Some("attachment; filename*=UTF-8''Box%20lid.dxf"));
        poll(&mut app);
        assert_eq!(app.session.doc.shapes.len(), 1);
        assert_eq!(status(&app).0, "Opened Box lid.dxf");
    }

    #[test]
    fn links_that_are_not_makerfoundrys_start_nothing() {
        for bad in [
            "http://makerfoundry.app/api/v1/designs/1/file/a.svg",
            "https://makerfoundry.app.evil.com/api/v1/designs/1/file/a.svg",
            "https://makerfoundry.app@evil.com/api/v1/designs/1/file/a.svg",
            "https://evil.com/api/v1/designs/1/file/a.svg",
            "https://makerfoundry.app:8443/api/v1/designs/1/file/a.svg",
            "https://makerfoundry.app/api/v1/../../login",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "",
        ] {
            let (mut app, shared) = app_opened_with(bad);
            poll(&mut app);
            assert!(shared.borrow().fetches.is_empty(), "{bad:?} must not be fetched");
            assert!(status(&app).1, "{bad:?} says why: {:?}", status(&app));
            assert!(app.session.doc.shapes.is_empty());
        }
    }

    #[test]
    fn every_refusal_is_explained_in_makerfoundrys_words_and_opens_nothing() {
        for (status_code, body, shown) in [
            (
                404,
                json!({"error": {"message": "That design isn't on MakerFoundry."}}).to_string(),
                "MakerFoundry says: That design isn't on MakerFoundry.",
            ),
            (403, json!({"error": {"message": "This design is for sale."}}).to_string(), "MakerFoundry says: This design is for sale."),
            (429, json!({"error": {"message": "Too many downloads."}}).to_string(), "MakerFoundry says: Too many downloads."),
            (500, "oops".to_string(), "The AI service failed (error 500): try again later."),
        ] {
            let (mut app, shared) = app_opened_with(LINK);
            poll(&mut app);
            fetched(&shared, status_code, body.as_bytes(), None);
            poll(&mut app);
            assert_eq!(status(&app), (shown.to_string(), true));
            assert!(app.session.doc.shapes.is_empty());
        }
    }

    #[test]
    fn a_file_that_is_not_a_design_or_is_broken_or_too_big_opens_nothing() {
        // the name says it is not an SVG or DXF
        let (mut app, shared) = app_opened_with("https://makerfoundry.app/api/v1/designs/1/file/noextension");
        poll(&mut app);
        fetched(&shared, 200, SVG.as_bytes(), Some("attachment; filename=\"rose.pes\""));
        poll(&mut app);
        assert_eq!(status(&app), ("That file isn't an SVG or DXF design, so it can't be opened here.".to_string(), true));
        assert!(app.session.doc.shapes.is_empty());
        // the name is fine, the file is rubbish
        for garbage in [&b"not an svg at all"[..], b"\x00\xff\xfe<svg<<<", b""] {
            let (mut app, shared) = app_opened_with(LINK);
            poll(&mut app);
            fetched(&shared, 200, garbage, None);
            poll(&mut app);
            assert!(status(&app).1, "{garbage:?}: {:?}", status(&app));
            assert!(app.session.doc.shapes.is_empty());
        }
        // larger than the cap
        let (mut app, shared) = app_opened_with(LINK);
        poll(&mut app);
        fetched(&shared, 200, &vec![b' '; maker::MAX_OPEN_BYTES + 1], None);
        poll(&mut app);
        assert_eq!(status(&app), ("That design is larger than this app opens from a link (32 MB).".to_string(), true));
    }

    #[test]
    fn a_failed_fetch_and_a_host_that_cannot_fetch_are_both_said_plainly() {
        let (mut app, shared) = app_opened_with(LINK);
        poll(&mut app);
        shared.borrow_mut().fetch_answers.push_back(Err("the connection was reset".into()));
        poll(&mut app);
        assert_eq!(status(&app), ("Couldn't reach the service: the connection was reset".to_string(), true));
        let (mut app, shared) = app_opened_with(LINK);
        shared.borrow_mut().can_fetch = false;
        poll(&mut app);
        assert_eq!(status(&app), ("This page can't open designs from a MakerFoundry link.".to_string(), true));
    }

    #[test]
    fn without_a_link_nothing_happens() {
        let (mut app, shared) = app_with_link(None);
        poll(&mut app);
        assert!(shared.borrow().fetches.is_empty());
        assert_eq!(app.status, None);
    }
}

mod publish_tests {
    use super::*;

    #[test]
    fn a_signed_in_person_sends_the_drawing_and_the_draft_page_opens() {
        let (mut app, shared) = signed_in_app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        app.maker.name_edited = Some("Rose box".into());
        app.start_publish();
        assert_eq!(app.ai.pending, Some(Pending::Publish));
        let sent = shared.borrow().sent.last().cloned().unwrap();
        assert_eq!(sent.url, "https://makerfoundry.app/api/v1/designs");
        assert!(sent.headers.iter().any(|(k, v)| *k == "authorization" && v == "Bearer sfs_tok"));
        let body: Value = serde_json::from_str(&sent.body).unwrap();
        assert_eq!(body["file_name"], "Rose box.svg");
        assert!(body["app"].as_str().unwrap().starts_with("LaserFoundry "));
        let svg = String::from_utf8(laserfoundry_doc::base64::decode(body["file"].as_str().unwrap()).unwrap()).unwrap();
        assert!(svg.contains("<svg") && svg.contains("<path") || svg.contains("<rect"), "the drawing as an SVG: {svg}");

        answer(&mut app, &shared, 201, r#"{"id": 42, "url": "https://makerfoundry.app/designs/42", "kind": "laser", "format": "SVG"}"#);
        assert_eq!(app.pending_url.as_deref(), Some("https://makerfoundry.app/designs/42"));
        assert!(!app.ai.note.as_ref().unwrap().1, "{:?}", app.ai.note);
    }

    #[test]
    fn nothing_is_sent_without_a_session_or_a_drawing() {
        let (mut app, shared) = app_with_secrets(json!({"consented": true}));
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
        app.open_ai(Window::Publish);
        app.start_publish();
        assert!(shared.borrow().sent.is_empty());
        assert!(app.ai.note.as_ref().unwrap().1, "it says to sign in");
        let (mut app, shared) = signed_in_app();
        app.start_publish();
        assert!(shared.borrow().sent.is_empty(), "an empty drawing is not sent");
        assert_eq!(app.ai.note, Some(("Draw something first: there is nothing to send.".to_string(), true)));
    }

    #[test]
    fn the_name_defaults_to_the_projects_and_what_the_person_types_wins() {
        let (mut app, shared) = signed_in_app();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
        let sent_name = |app: &mut LaserFoundryApp| {
            shared.borrow_mut().in_flight = false;
            app.ai.pending = None;
            let before = shared.borrow().sent.len();
            app.start_publish();
            let sent = shared.borrow().sent.get(before).cloned(); // only a request this call made
            sent.map(|r| serde_json::from_str::<Value>(&r.body).unwrap()["file_name"].as_str().unwrap().to_string())
        };
        // a project that has not been saved has no name to offer, so nothing is sent until the person gives one
        assert_eq!(app.publish_name(), "");
        assert_eq!(sent_name(&mut app), None);
        assert_eq!(app.ai.note, Some(("Give the design a name first: it becomes the draft's title on MakerFoundry.".to_string(), true)));
        // a saved project offers its own name, and follows it
        app.session.path = Some(PathBuf::from("/home/me/My Box (v2).laserfoundry"));
        assert_eq!(app.publish_name(), "My Box _v2");
        assert_eq!(sent_name(&mut app).as_deref(), Some("My Box _v2.svg"));
        app.session.path = Some(PathBuf::from("/home/me/Lid.laserfoundry"));
        assert_eq!(sent_name(&mut app).as_deref(), Some("Lid.svg"));
        // what the person types wins, and stays when the project changes
        app.maker.name_edited = Some("Wedding sign – final!".into());
        assert_eq!(sent_name(&mut app).as_deref(), Some("Wedding sign _ final.svg"));
        app.session.path = Some(PathBuf::from("/home/me/Other.laserfoundry"));
        assert_eq!(sent_name(&mut app).as_deref(), Some("Wedding sign _ final.svg"));
        // emptying the field means "no name", not "use the project's"
        app.maker.name_edited = Some(String::new());
        assert_eq!(sent_name(&mut app), None);
    }

    #[test]
    fn a_name_is_made_plain_and_never_empty_or_a_path() {
        for (typed, plain) in [
            ("Box lid", "Box lid"),
            ("  Box lid  ", "Box lid"),
            ("Box_lid-v2", "Box_lid-v2"),
            ("My Box (v2)", "My Box _v2"),
            ("..hidden..", "hidden"),
            ("a/b\\c", "a_b_c"),
            ("../../etc/passwd", "etc_passwd"),
            ("évè-1 ünï", "évè-1 ünï"),
            ("花瓶", "花瓶"),
            ("a\nb\tc\u{0}d", "a_b_c_d"),
            ("name.svg", "name"),
            ("NAME.SVG", "NAME"),
            ("a.svg.svg", "a_svg"),
            (".svg", ""),
            ("svg", "svg"),
            ("花瓶.svg", "花瓶"),
            ("___", ""),
            ("...", ""),
            ("   ", ""),
            ("🙂", ""),
            ("", ""),
        ] {
            assert_eq!(plain_design_name(typed), plain, "{typed:?}");
        }
        let long = "x".repeat(200);
        assert_eq!(plain_design_name(&long).chars().count(), MAX_NAME);
        // whatever the person types, what is sent is accepted by the checks MakerFoundry's address and the request have
        let (mut app, shared) = ready_app();
        for typed in ["Box lid", "../../etc/passwd", "a\u{0}b", "name.svg", &long, "花瓶", "é.svg", "\u{1F642}ab.SVG"] {
            shared.borrow_mut().in_flight = false;
            app.ai.pending = None;
            app.maker.name_edited = Some(typed.to_string());
            app.start_publish();
            let sent = shared.borrow().sent.last().cloned().unwrap();
            let name = serde_json::from_str::<Value>(&sent.body).unwrap()["file_name"].as_str().unwrap().to_string();
            assert!(name.ends_with(".svg") && !name.contains(['/', '\\']) && name.chars().count() <= MAX_NAME + 4, "{typed:?} -> {name:?}");
        }
    }

    #[test]
    fn answers_that_misdirect_or_refuse_are_handled() {
        // a draft address on another site is never opened
        for url in ["https://evil.com/designs/1", "http://makerfoundry.app/designs/1", "javascript:alert(1)", "https://makerfoundry.app@evil.com/x"] {
            let (mut app, shared) = ready_app();
            app.start_publish();
            answer(&mut app, &shared, 201, &json!({"id": 1, "url": url}).to_string());
            assert_eq!(app.pending_url, None, "{url}");
            assert!(app.ai.note.as_ref().unwrap().1);
        }
        // MakerFoundry's own sentence is shown as it is
        let (mut app, shared) = ready_app();
        app.start_publish();
        answer(&mut app, &shared, 429, r#"{"error": {"message": "Too many designs today."}}"#);
        assert_eq!(app.ai.note, Some(("MakerFoundry says: Too many designs today.".to_string(), true)));
        // an ended session is forgotten, so the window asks for a sign-in
        let (mut app, shared) = ready_app();
        app.start_publish();
        answer(&mut app, &shared, 401, r#"{"error": {"message": "Sign in to your Foundry Account first."}}"#);
        assert_eq!(app.ai.token, None);
        assert!(!shared.borrow().secrets.values().any(|v| v.contains("sfs_tok")), "the dead token is no longer kept");
    }

    #[test]
    fn a_second_send_while_one_is_running_is_refused_not_queued() {
        let (mut app, shared) = ready_app();
        app.start_publish();
        app.ai.pending = None; // as if the window let a second click through
        app.start_publish();
        assert_eq!(shared.borrow().sent.len(), 1);
        assert!(app.ai.note.as_ref().unwrap().1);
    }
}

#[derive(Default)]
struct Shared {
    link: Option<String>,
    /// Every fetch the app started: the address and the size cap.
    fetches: Vec<(String, usize)>,
    fetch_answers: VecDeque<Result<Fetched, String>>,
    can_fetch: bool,
    sent: Vec<HttpRequest>,
    answers: VecDeque<Result<(u16, String), String>>,
    in_flight: bool,
    secrets: HashMap<String, String>,
}

struct Fake(Rc<RefCell<Shared>>);

impl Platform for Fake {
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
        Err("no serial".into())
    }

    fn http_available(&self) -> bool {
        true
    }

    fn startup_link(&self) -> Option<String> {
        self.0.borrow().link.clone()
    }

    fn start_fetch(&self, url: &str, max_bytes: usize) -> bool {
        let mut s = self.0.borrow_mut();
        if !s.can_fetch {
            return false;
        }
        s.fetches.push((url.to_string(), max_bytes));
        true
    }

    fn take_fetch(&self) -> Option<Result<Fetched, String>> {
        self.0.borrow_mut().fetch_answers.pop_front()
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

fn app_with_link(link: Option<&str>) -> (LaserFoundryApp, Rc<RefCell<Shared>>) {
    build(Shared { link: link.map(Into::into), can_fetch: true, ..Shared::default() })
}

fn app_opened_with(link: &str) -> (LaserFoundryApp, Rc<RefCell<Shared>>) {
    app_with_link(Some(link))
}

/// An app that starts with these settings already kept (the app reads them when it starts).
fn app_with_secrets(secrets: Value) -> (LaserFoundryApp, Rc<RefCell<Shared>>) {
    let mut shared = Shared { can_fetch: true, ..Shared::default() };
    shared.secrets.insert("laserfoundry.ai".into(), secrets.to_string());
    build(shared)
}

fn build(shared: Shared) -> (LaserFoundryApp, Rc<RefCell<Shared>>) {
    let shared = Rc::new(RefCell::new(shared));
    (LaserFoundryApp::new(Session::new(), Box::new(Fake(Rc::clone(&shared)))), shared)
}

/// A signed-in app with a drawing and a name for it, ready to send.
fn ready_app() -> (LaserFoundryApp, Rc<RefCell<Shared>>) {
    let (mut app, shared) = signed_in_app();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    app.maker.name_edited = Some("Test design".into());
    (app, shared)
}

/// An app whose person is signed in to a Foundry Account.
fn signed_in_app() -> (LaserFoundryApp, Rc<RefCell<Shared>>) {
    let (mut app, shared) =
        app_with_secrets(json!({"consented": true, "server": "https://c.example", "token": "sfs_tok", "email": "me@example.com"}));
    app.open_ai(Window::Publish);
    (app, shared)
}

fn poll(app: &mut LaserFoundryApp) {
    app.poll_maker(&egui::Context::default());
}

fn fetched(shared: &Rc<RefCell<Shared>>, status: u16, bytes: &[u8], disposition: Option<&str>) {
    shared.borrow_mut().fetch_answers.push_back(Ok(Fetched { status, bytes: bytes.to_vec(), content_disposition: disposition.map(Into::into) }));
}

/// Gives the answer to the request in flight and lets the app read it.
fn answer(app: &mut LaserFoundryApp, shared: &Rc<RefCell<Shared>>, status: u16, body: &str) {
    shared.borrow_mut().answers.push_back(Ok((status, body.to_string())));
    app.poll_ai(&egui::Context::default());
}

fn status(app: &LaserFoundryApp) -> (String, bool) {
    app.status.clone().unwrap_or_default()
}
