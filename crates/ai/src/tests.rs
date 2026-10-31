use serde_json::{Value, json};

use super::art::{self, ArtRequest, Use};
use super::check::{self, CheckRequest};
use super::maker;
use super::*;

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80mm" height="40mm" viewBox="0 0 80 40"><rect width="80" height="40" fill="none" stroke="#ff0000"/></svg>"##;

fn reply(stop: &str, text: &str) -> String {
    json!({"type": "message", "model": MODEL, "stop_reason": stop, "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": text}]})
        .to_string()
}

fn header<'a>(r: &'a HttpRequest, name: &str) -> Option<&'a str> {
    r.headers.iter().find(|h| h.0 == name).map(|h| h.1.as_str())
}

fn key() -> Auth {
    Auth::Key(" sk-ant-test ".into())
}

fn session() -> Auth {
    Auth::Session { server: "https://c.example/".into(), token: "sfs_abc".into() }
}

mod request_tests {
    use super::*;

    #[test]
    fn a_key_sends_the_request_straight_to_anthropic() {
        let http = ArtRequest::new("  a fox head  ", 80.0, Use::CutAndEngrave).http(&key()).unwrap();
        assert_eq!(http.url, API_URL);
        assert_eq!(header(&http, "x-api-key"), Some("sk-ant-test"));
        assert_eq!(header(&http, "anthropic-version"), Some(API_VERSION));
        assert_eq!(header(&http, "authorization"), None);
        let b: Value = serde_json::from_str(&http.body).unwrap();
        assert_eq!(b["model"], MODEL);
        assert_eq!(b["max_tokens"], MAX_TOKENS);
        assert!(b["messages"][0]["content"].as_str().unwrap().ends_with("a fox head"));
        let system = b["system"].as_str().unwrap();
        assert!(
            system.contains("about 80 mm") && system.contains("#ff0000") && system.contains("#000000") && system.contains("DECLINED:"),
            "{system}"
        );
    }

    #[test]
    fn a_session_goes_through_the_service_and_never_sends_the_key_there() {
        let http = ArtRequest::new("a star", 50.0, Use::Cut).http(&session()).unwrap();
        assert_eq!(http.url, "https://c.example/v1/messages");
        assert_eq!(header(&http, "authorization"), Some("Bearer sfs_abc"));
        assert_eq!(header(&http, "x-api-key"), None);
        // an Anthropic key in a session is refused: it would be handed to the service
        let own = Auth::Session { server: "https://c.example".into(), token: "sk-ant-mine".into() };
        assert_eq!(ArtRequest::new("a star", 50.0, Use::Cut).http(&own), Err(AiError::SignIn));
        let no_server = Auth::Session { server: "".into(), token: "sfs_abc".into() };
        assert_eq!(ArtRequest::new("a star", 50.0, Use::Cut).http(&no_server), Err(AiError::NoServer));
    }

    #[test]
    fn the_use_changes_what_the_model_is_told() {
        let cut = ArtRequest::new("a", 50.0, Use::Cut).system_prompt();
        assert!(cut.contains("Nothing is engraved") && !cut.contains("#0000ff"), "{cut}");
        let engrave = ArtRequest::new("a", 50.0, Use::Engrave).system_prompt();
        assert!(engrave.contains("Nothing is cut through"), "{engrave}");
        let both = ArtRequest::new("a", 50.0, Use::CutAndEngrave).system_prompt();
        assert!(both.contains("#0000ff") && both.contains("#ff0000") && both.contains("#000000"), "{both}");
    }

    #[test]
    fn a_redrawing_sends_the_picture_first_and_needs_no_description() {
        let pic = Picture { media_type: "image/png", data_base64: "iVBORw0KGgo=".into() };
        let r = ArtRequest::new("", 70.0, Use::CutAndEngrave).with_image(pic.clone());
        let b: Value = serde_json::from_str(&r.http(&key()).unwrap().body).unwrap();
        let content = b["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["text"], "Redraw this picture as laser artwork.");
        assert!(!b["system"].as_str().unwrap().contains("DECLINED"));
        let r = ArtRequest::new("drop the border", 70.0, Use::Engrave).with_image(pic);
        let b: Value = serde_json::from_str(&r.http(&key()).unwrap().body).unwrap();
        assert!(b["messages"][0]["content"][1]["text"].as_str().unwrap().ends_with("What to change: drop the border"));
    }

    #[test]
    fn requests_are_checked_before_sending() {
        let r = ArtRequest::new("a star", 100.0, Use::Cut);
        assert_eq!(r.http(&Auth::Key("  ".into())), Err(AiError::NoKey));
        assert_eq!(r.http(&Auth::Key("sk-ant bad".into())), Err(AiError::BadKey));
        assert_eq!(r.http(&Auth::Key("sk-ant-\r\nx".into())), Err(AiError::BadKey));
        assert_eq!(ArtRequest::new(" ", 100.0, Use::Cut).http(&key()), Err(AiError::NoPrompt));
        assert_eq!(ArtRequest::new(&"x".repeat(MAX_TEXT + 1), 100.0, Use::Cut).http(&key()), Err(AiError::TooLong));
        let big = Picture { media_type: "image/jpeg", data_base64: "A".repeat(MAX_IMAGE_BASE64 + 1) };
        assert_eq!(ArtRequest::new("", 70.0, Use::Cut).with_image(big).http(&key()), Err(AiError::LargePicture));
        // sizes come into range
        assert_eq!(ArtRequest::new("a", f64::NAN, Use::Cut).size_mm, 100.0);
        assert_eq!(ArtRequest::new("a", 1e9, Use::Cut).size_mm, art::SIZE_RANGE.1);
        assert_eq!(ArtRequest::new("a", -5.0, Use::Cut).size_mm, art::SIZE_RANGE.0);
    }
}

mod art_reply_tests {
    use super::*;

    #[test]
    fn the_svg_is_read_from_text_around_thinking_and_fences() {
        let body = reply("end_turn", &format!("Here it is:\n```svg\n<?xml version=\"1.0\"?>\n{SVG}\n```"));
        let art = art::parse_reply(200, &body, false).unwrap();
        assert_eq!(art.svg, SVG);
        assert_eq!(art.model, MODEL);
    }

    #[test]
    fn stop_reasons_and_missing_art_are_errors() {
        assert_eq!(art::parse_reply(200, &reply("refusal", "x"), false), Err(AiError::Refused));
        assert_eq!(art::parse_reply(200, &reply("max_tokens", "<svg><path d=\"M0 0"), false), Err(AiError::Truncated));
        assert_eq!(art::parse_reply(200, &reply("end_turn", "I can't draw that."), false), Err(AiError::NoArt));
        assert_eq!(art::parse_reply(200, "not json", false), Err(AiError::NoArt));
        let declined = reply("end_turn", "DECLINED: that is a sports team's logo");
        assert_eq!(art::parse_reply(200, &declined, false), Err(AiError::NotOriginal("that is a sports team's logo".into())));
        let bare = reply("end_turn", "DECLINED:");
        assert!(matches!(art::parse_reply(200, &bare, false), Err(AiError::NotOriginal(r)) if !r.is_empty()));
        let huge = format!("<svg>{}</svg>", "x".repeat(art::MAX_SVG));
        assert_eq!(art::parse_reply(200, &reply("end_turn", &huge), false), Err(AiError::NoArt));
    }

    #[test]
    fn http_errors_mean_different_things_with_a_key_and_with_the_service() {
        let err = |msg: &str| json!({"type": "error", "error": {"type": "x", "message": msg}}).to_string();
        assert_eq!(art::parse_reply(401, &err("invalid x-api-key"), false), Err(AiError::Key("invalid x-api-key".into())));
        assert_eq!(art::parse_reply(401, &err("expired"), true), Err(AiError::SignIn));
        assert_eq!(art::parse_reply(402, &err("low"), false), Err(AiError::Billing("low".into())));
        assert_eq!(art::parse_reply(402, &err("low"), true), Err(AiError::NoCredits));
        assert_eq!(art::parse_reply(429, &err("slow"), true), Err(AiError::Busy));
        assert_eq!(art::parse_reply(529, &err("busy"), false), Err(AiError::Busy));
        assert_eq!(art::parse_reply(503, &err("off"), true), Err(AiError::CloudClosed));
        assert_eq!(art::parse_reply(500, &err("oops"), false), Err(AiError::Service(500)));
        assert_eq!(art::parse_reply(400, &err("bad field"), false), Err(AiError::Request(400, "bad field".into())));
        assert_eq!(art::parse_reply(404, "<html>gone</html>", false), Err(AiError::Request(404, "<html>gone</html>".into())));
        assert_eq!(art::parse_reply(413, "", true), Err(AiError::Request(413, "no details".into())));
        assert!(AiError::Busy.retryable() && AiError::Network("x".into()).retryable() && !AiError::Refused.retryable());
    }
}

mod ask_tests {
    use super::*;
    use crate::ask::{self, Turn};

    fn person(t: &str) -> Turn {
        Turn { from_person: true, text: t.into() }
    }

    fn model(t: &str) -> Turn {
        Turn { from_person: false, text: t.into() }
    }

    #[test]
    fn a_question_needs_text_and_a_reasonable_length() {
        assert_eq!(ask::question("   "), Err(AiError::NoQuestion));
        assert_eq!(ask::question(&"x".repeat(MAX_TEXT + 1)), Err(AiError::TooLong));
        assert_eq!(ask::question("  How slow for 3 mm ply? ").unwrap().text, "How slow for 3 mm ply?");
    }

    #[test]
    fn the_document_is_context_not_instructions_and_is_bounded() {
        let http = ask::request(&key(), "Layer 0: line, 1000 mm/min, 60 %", &[person("Is that right?")]).unwrap();
        let b: Value = serde_json::from_str(&http.body).unwrap();
        let system = b["system"].as_str().unwrap();
        assert!(system.contains("Layer 0: line, 1000 mm/min") && system.contains("not instructions"), "{system}");
        assert_eq!(b["messages"][0], json!({"role": "user", "content": "Is that right?"}));
        let long = ask::system_prompt(&"y".repeat(ask::MAX_CONTEXT * 2));
        assert!(long.chars().count() < ask::MAX_CONTEXT + 2_000);
    }

    #[test]
    fn a_conversation_alternates_starts_with_a_question_and_ends_with_one() {
        let turns = [person("one"), model("a1"), person("two"), person("two again"), model("a2"), person("three")];
        let b = ask::request_body("", &turns).unwrap();
        let roles: Vec<&str> = b["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user", "assistant", "user"]);
        assert_eq!(b["messages"][2]["content"], "two\n\ntwo again");
        assert_eq!(ask::request_body("", &[person("q"), model("a")]).err(), Some(AiError::NoQuestion));
        assert_eq!(ask::request_body("", &[]).err(), Some(AiError::NoQuestion));
        // only the newest turns go, and the first of them is a question
        let many: Vec<Turn> = (0..40).flat_map(|i| [person(&format!("q{i}")), model(&format!("a{i}"))]).chain([person("last")]).collect();
        let b = ask::request_body("", &many).unwrap();
        let sent = b["messages"].as_array().unwrap();
        assert!(sent.len() <= ask::MAX_TURNS && sent[0]["role"] == "user" && sent.last().unwrap()["content"] == "last");
    }

    #[test]
    fn the_answer_is_the_text() {
        assert_eq!(ask::parse_reply(200, &reply("end_turn", "  Try 1500 mm/min.  "), false).unwrap(), "Try 1500 mm/min.");
        assert_eq!(ask::parse_reply(200, &reply("end_turn", "   "), false), Err(AiError::NoAnswer));
        assert_eq!(ask::parse_reply(200, &reply("refusal", "x"), false), Err(AiError::Refused));
    }
}

mod check_tests {
    use super::*;

    fn photo() -> Picture {
        Picture { media_type: "image/jpeg", data_base64: "/9j/4AAQ".into() }
    }

    fn check_reply(json_text: &str) -> String {
        reply("end_turn", json_text)
    }

    #[test]
    fn a_check_sends_the_photos_then_the_notes_and_settings() {
        let r = CheckRequest {
            photos: vec![photo(), photo()],
            notes: "3 mm birch ply, edges charred".into(),
            settings: "Layer 2: line 600 mm/min 80 %".into(),
        };
        let b: Value = serde_json::from_str(&r.http(&session()).unwrap().body).unwrap();
        let content = b["messages"][0]["content"].as_array().unwrap();
        assert_eq!(
            (content[0]["type"].as_str(), content[1]["type"].as_str(), content[2]["type"].as_str()),
            (Some("image"), Some("image"), Some("text"))
        );
        let text = content[2]["text"].as_str().unwrap();
        assert!(text.contains("3 mm birch ply") && text.contains("Layer 2: line 600 mm/min 80 %"), "{text}");
        assert!(b["system"].as_str().unwrap().contains("not instructions"));
        let none = CheckRequest { photos: vec![], notes: String::new(), settings: String::new() };
        assert_eq!(none.http(&key()), Err(AiError::NoPhoto));
        let many = CheckRequest { photos: vec![photo(); MAX_IMAGES + 1], notes: String::new(), settings: String::new() };
        assert_eq!(many.http(&key()), Err(AiError::LargePicture));
    }

    #[test]
    fn suggestions_are_kept_only_when_they_make_sense() {
        let text = r#"Sure:
        {"summary": "Edges are charred: go faster.", "suggestions": [
          {"layer": 2, "speed": 1200, "max_power": 70, "passes": 1, "reason": "less heat"},
          {"layer": 99, "speed": 1200},
          {"layer": 1, "speed": -5},
          {"layer": 1, "max_power": 150},
          {"layer": 1, "passes": 0},
          {"layer": 1, "speed": "fast"},
          {"layer": 3},
          {"layer": 4, "passes": 2}
        ]}"#;
        let c = check::parse_reply(200, &check_reply(text), false).unwrap();
        assert_eq!(c.summary, "Edges are charred: go faster.");
        assert_eq!(c.suggestions.len(), 2, "{:?}", c.suggestions);
        assert_eq!(
            c.suggestions[0],
            check::Suggestion { layer: 2, speed: Some(1200.0), max_power: Some(70.0), passes: Some(1), reason: "less heat".into() }
        );
        assert_eq!((c.suggestions[1].layer, c.suggestions[1].passes, c.suggestions[1].speed), (4, Some(2), None));
    }

    #[test]
    fn unreadable_answers_are_errors_not_guesses() {
        for bad in ["no json here", "{\"summary\": 3}", "{\"suggestions\": []}", "{\"summary\": \"  \"}", "{not json}"] {
            assert_eq!(check::parse_reply(200, &check_reply(bad), false), Err(AiError::NoAnswer), "{bad}");
        }
        let many = format!("{{\"summary\": \"s\", \"suggestions\": [{}]}}", vec![r#"{"layer":0,"passes":2}"#; 100].join(","));
        assert_eq!(check::read_check(&many).unwrap().suggestions.len(), check::MAX_SUGGESTIONS);
    }
}

mod cloud_tests {
    use super::*;
    use crate::cloud;

    #[test]
    fn only_https_or_this_computer() {
        assert_eq!(cloud::clean_server(" https://cloud.example.com/ ").as_deref(), Some("https://cloud.example.com"));
        assert_eq!(cloud::clean_server("http://127.0.0.1:8080").as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(cloud::clean_server("http://localhost").as_deref(), Some("http://localhost"));
        for bad in ["", "https://", "http://cloud.example.com", "ftp://x", "http://localhost.evil.com", "https://a b.com", "javascript:alert(1)"] {
            assert_eq!(cloud::clean_server(bad), None, "{bad}");
        }
    }

    #[test]
    fn account_requests_and_answers() {
        let r = cloud::sign_in("https://c.example", " denise@example.com ", "fire truck 1").unwrap();
        assert_eq!(r.url, "https://c.example/v1/signin");
        assert_eq!(serde_json::from_str::<Value>(&r.body).unwrap(), json!({"email": "denise@example.com", "password": "fire truck 1"}));
        assert_eq!(header(&r, "authorization"), None);
        let r = cloud::checkout("https://c.example", "sfs_abc", "small").unwrap();
        assert_eq!(header(&r, "authorization"), Some("Bearer sfs_abc"));
        assert_eq!(cloud::account("", "sfs_abc"), Err(AiError::NoServer));
        assert_eq!(cloud::account("https://c.example", "bad token"), Err(AiError::SignIn));
        assert_eq!(cloud::account("https://c.example", &"s".repeat(500)), Err(AiError::SignIn));
        assert_eq!(cloud::parse(402, r#"{"error":{"message":"x"}}"#), Err(AiError::NoCredits));
        assert_eq!(cloud::parse(409, "{}"), Err(AiError::Taken));
        assert_eq!(cloud::parse(200, "oops"), Err(AiError::NoAnswer));
        assert_eq!(
            cloud::parse(400, r#"{"error":{"message":"a password needs at least 8 characters"}}"#),
            Err(AiError::Request(400, "a password needs at least 8 characters".into()))
        );
    }

    #[test]
    fn a_session_and_an_account_are_read_defensively() {
        let a = json!({"token": "sfs_xyz", "account": {"email": "a@b.c", "credits": 100}});
        assert_eq!(cloud::session(&a), Some(("sfs_xyz".into(), "a@b.c".into())));
        assert_eq!(cloud::session(&json!({"token": "sk-ant-oops"})), None);
        assert_eq!(cloud::session(&json!({"token": 5})), None);
        let acct = cloud::read_account(&json!({"email": "a@b.c", "credits": 42, "payments": true, "packs": [
            {"id": "small", "credits": 1000, "cents": 1000}, {"id": "", "credits": 1}, {"credits": 2}, {"id": "big", "credits": -4, "cents": "x"}]}));
        assert_eq!((acct.credits, acct.payments, acct.packs.len()), (42, true, 2));
        assert_eq!(acct.packs[0], cloud::Pack { id: "small".into(), credits: 1000, cents: 1000 });
        assert_eq!((acct.packs[1].credits, acct.packs[1].cents), (0, 0));
        assert_eq!(cloud::read_account(&json!("junk")), cloud::Account::default());
        assert_eq!(cloud::dollars(1000), "$10.00");
        assert_eq!(cloud::dollars(2705), "$27.05");
    }

    #[test]
    fn the_built_in_service_is_an_https_origin_and_requests_go_to_it() {
        let server = cloud::built_in_server();
        assert!(server.starts_with("https://"), "{server}");
        assert!(!server.ends_with('/'), "{server}");
        let request = cloud::sign_in(server, "a@b.c", "password1").unwrap();
        assert_eq!(request.url, format!("{server}/v1/signin"));
    }

    #[test]
    fn only_https_checkout_pages_are_opened() {
        assert!(cloud::checkout_url(&json!({"url": "https://checkout.stripe.com/c/pay/x"})).is_some());
        for bad in ["http://x", "javascript:alert(1)", "file:///etc/passwd", "https://a b", ""] {
            assert_eq!(cloud::checkout_url(&json!({ "url": bad })), None, "{bad}");
        }
        assert_eq!(cloud::checkout_url(&json!({})), None);
    }
}

mod maker_tests {
    use super::*;

    mod open_link_tests {
        use super::*;

        #[test]
        fn a_design_file_on_makerfoundry_is_accepted_and_named() {
            let link = maker::open_link("https://makerfoundry.app/api/v1/designs/42/file/rose.svg").unwrap();
            assert_eq!(link.url, "https://makerfoundry.app/api/v1/designs/42/file/rose.svg");
            assert_eq!(link.name.as_deref(), Some("rose.svg"));
            let link = maker::open_link("https://www.makerfoundry.app/api/v1/designs/7/file/Box%20lid.DXF?t=abc123").unwrap();
            assert_eq!(link.url, "https://www.makerfoundry.app/api/v1/designs/7/file/Box%20lid.DXF?t=abc123", "a buyer's link keeps its token");
            assert_eq!(link.name.as_deref(), Some("Box lid.DXF"));
            assert_eq!(
                maker::open_link(" https://MakerFoundry.app/api/v1/designs/1/file/a.svg#frag ").unwrap().url,
                "https://makerfoundry.app/api/v1/designs/1/file/a.svg"
            );
        }

        #[test]
        fn look_alike_hosts_and_hidden_hosts_are_refused() {
            for bad in [
                "http://makerfoundry.app/api/v1/designs/1/file/a.svg",
                "//makerfoundry.app/api/v1/designs/1/file/a.svg",
                "https://makerfoundry.app.evil.com/api/v1/designs/1/file/a.svg",
                "https://evil.com/makerfoundry.app/api/v1/designs/1/file/a.svg",
                "https://evil.com/?https://makerfoundry.app/api/v1/x.svg",
                "https://makerfoundry.app@evil.com/api/v1/designs/1/file/a.svg",
                "https://user@makerfoundry.app/api/v1/designs/1/file/a.svg",
                "https://user:pw@makerfoundry.app/api/v1/designs/1/file/a.svg",
                "https://makerfoundry.app:8443/api/v1/designs/1/file/a.svg",
                "https://makerfoundry.app:443/api/v1/designs/1/file/a.svg",
                "https://makerfoundry.app./api/v1/designs/1/file/a.svg",
                "https://evilmakerfoundry.app/api/v1/designs/1/file/a.svg",
                "https://sub.makerfoundry.app/api/v1/designs/1/file/a.svg",
                "https://makerfoundry.app\\@evil.com/api/v1/x.svg",
                "https://makerfoundry.app%2f@evil.com/api/v1/x.svg",
                "https://m\u{430}kerfoundry.app/api/v1/designs/1/file/a.svg",
                "https://[::1]/api/v1/designs/1/file/a.svg",
                "https://127.0.0.1/api/v1/designs/1/file/a.svg",
                "javascript:alert(1)",
                "data:image/svg+xml,<svg/>",
                "file:///etc/passwd",
                "ftp://makerfoundry.app/api/v1/x.svg",
                "makerfoundry.app/api/v1/designs/1/file/a.svg",
            ] {
                assert_eq!(maker::open_link(bad), Err(AiError::BadLink), "{bad}");
            }
        }

        #[test]
        fn control_characters_spaces_and_paths_that_leave_the_api_are_refused() {
            for bad in [
                "",
                "   ",
                "https://makerfoundry.app",
                "https://makerfoundry.app/",
                "https://makerfoundry.app?x=/api/v1/a.svg",
                "https://makerfoundry.app/login",
                "https://makerfoundry.app/designs/1",
                "https://makerfoundry.app/api/v1",
                "https://makerfoundry.app/api/v1/../../admin/a.svg",
                "https://makerfoundry.app/api/v1/designs/1/file/..",
                "https://makerfoundry.app/api/v1/designs/./1/../../../x.svg",
                "https://makerfoundry.app/api/v1/designs/1/file/a b.svg",
                "https://makerfoundry.app/api/v1/designs/1/file/a\tb.svg",
                "https://makerfoundry.app/api/v1/designs/1/file/a\nb.svg",
                "https://makerfoundry.app/api/v1/designs/1/file/a\u{0}b.svg",
                "https://makerfoundry.app/api/v1/designs/1/file/\u{e9}.svg",
            ] {
                assert_eq!(maker::open_link(bad), Err(AiError::BadLink), "{bad:?}");
            }
            let long = format!("https://makerfoundry.app/api/v1/designs/1/file/{}.svg", "a".repeat(3000));
            assert_eq!(maker::open_link(&long), Err(AiError::BadLink));
        }

        #[test]
        fn a_bad_file_name_does_not_make_the_link_bad_but_gives_no_name() {
            for tail in ["noextension", "a.pes", "a.svg.exe", ".svg", "%zz.svg", "%ff%fe.svg", "%00.svg"] {
                let link = maker::open_link(&format!("https://makerfoundry.app/api/v1/designs/1/file/{tail}")).unwrap();
                assert_eq!(link.name, None, "{tail}");
            }
            // an encoded slash is cut to the last part, never a path
            let link = maker::open_link("https://makerfoundry.app/api/v1/designs/1/file/..%2f..%2fetc%2fa.svg").unwrap();
            assert_eq!(link.name.as_deref(), Some("a.svg"));
        }
    }

    mod file_name_tests {
        use super::*;

        fn link(name: Option<&str>) -> maker::OpenLink {
            maker::OpenLink { url: "https://makerfoundry.app/api/v1/designs/1/file/x".into(), name: name.map(Into::into) }
        }

        #[test]
        fn the_header_wins_and_the_address_is_the_fallback() {
            assert_eq!(maker::file_name(&link(Some("a.svg")), Some("attachment; filename=\"rose.dxf\"")).as_deref(), Ok("rose.dxf"));
            assert_eq!(maker::file_name(&link(Some("a.svg")), Some("attachment; filename*=UTF-8''Box%20lid.svg")).as_deref(), Ok("Box lid.svg"));
            assert_eq!(maker::file_name(&link(Some("a.svg")), Some("attachment; FILENAME=b.SVG")).as_deref(), Ok("b.SVG"));
            assert_eq!(maker::file_name(&link(Some("a.svg")), None).as_deref(), Ok("a.svg"));
            assert_eq!(maker::file_name(&link(Some("a.svg")), Some("inline")).as_deref(), Ok("a.svg"));
        }

        #[test]
        fn a_header_that_is_not_a_design_or_tries_a_path_is_not_believed() {
            for header in [
                "attachment; filename=\"rose.pes\"",
                "attachment; filename=\"\"",
                "attachment; filename*=ISO-8859-1''a.svg",
                "attachment; filename*=UTF-8''%zz.svg",
                "attachment; filename=\"x\u{0}.svg\"",
                ";;;;",
                "filename",
            ] {
                assert_eq!(maker::file_name(&link(None), Some(header)), Err(AiError::NotADesign), "{header}");
            }
            assert_eq!(maker::file_name(&link(None), Some("attachment; filename=\"../../etc/a.svg\"")).as_deref(), Ok("a.svg"));
            assert_eq!(maker::file_name(&link(Some("ok.svg")), Some("attachment; filename=\"../../etc/passwd\"")).as_deref(), Ok("ok.svg"));
            assert_eq!(maker::file_name(&link(None), None), Err(AiError::NotADesign));
            let huge = format!("attachment; filename=\"{}.svg\"", "a".repeat(5000));
            assert_eq!(maker::file_name(&link(None), Some(&huge)), Err(AiError::NotADesign));
        }
    }

    mod publish_tests {
        use super::*;

        #[test]
        fn a_design_is_sent_to_the_designs_address_with_the_session_token() {
            let r = maker::publish("https://makerfoundry.app/", " sfs_tok ", "rose.svg", "PHN2Zy8+", "LaserFoundry 0.2.0").unwrap();
            assert_eq!(r.url, "https://makerfoundry.app/api/v1/designs");
            assert_eq!(header(&r, "authorization"), Some("Bearer sfs_tok"));
            assert_eq!(header(&r, "content-type"), Some("application/json"));
            let b: Value = serde_json::from_str(&r.body).unwrap();
            assert_eq!(b, json!({"file_name": "rose.svg", "file": "PHN2Zy8+", "app": "LaserFoundry 0.2.0"}));
        }

        #[test]
        fn only_a_foundry_session_may_send_and_only_a_design_file() {
            assert_eq!(
                maker::publish("https://makerfoundry.app", "sk-ant-key", "a.svg", "AAAA", "x"),
                Err(AiError::SignIn),
                "an Anthropic key is never sent"
            );
            assert_eq!(maker::publish("https://makerfoundry.app", "", "a.svg", "AAAA", "x"), Err(AiError::SignIn));
            for name in ["a.pes", "a", "", ".svg", "a.svg.png"] {
                assert_eq!(maker::publish("https://makerfoundry.app", "sfs_t", name, "AAAA", "x"), Err(AiError::NotADesign), "{name}");
            }
            assert_eq!(maker::publish("https://makerfoundry.app", "sfs_t", "a.svg", "", "x"), Err(AiError::NotADesign));
            assert_eq!(
                maker::publish("http://makerfoundry.app", "sfs_t", "a.svg", "AAAA", "x"),
                Err(AiError::NoServer),
                "plain http only to this computer"
            );
            let local = maker::publish("http://127.0.0.1:8080", "sfs_t", "a.svg", "AAAA", "x").unwrap();
            assert_eq!(local.url, "http://127.0.0.1:8080/api/v1/designs");
        }

        #[test]
        fn the_name_is_a_plain_name_and_a_big_file_is_refused_before_sending() {
            let r = maker::publish("https://makerfoundry.app", "sfs_t", "../../etc/rose.svg", "AAAA", "x").unwrap();
            assert_eq!(serde_json::from_str::<Value>(&r.body).unwrap()["file_name"], "rose.svg");
            let r = maker::publish("https://makerfoundry.app", "sfs_t", "a.svg", "AAAA", "App\r\nX-Evil: 1").unwrap();
            assert_eq!(serde_json::from_str::<Value>(&r.body).unwrap()["app"], "AppX-Evil: 1", "control characters are dropped");
            let big = "A".repeat(maker::MAX_PUBLISH_BYTES / 3 * 4 + 8);
            assert_eq!(maker::publish("https://makerfoundry.app", "sfs_t", "a.svg", &big, "x"), Err(AiError::TooBigToPublish));
            let fits = "A".repeat(maker::MAX_PUBLISH_BYTES / 3 * 4);
            assert!(maker::publish("https://makerfoundry.app", "sfs_t", "a.svg", &fits, "x").is_ok());
        }
    }

    mod answer_tests {
        use super::*;

        const SERVER: &str = "https://makerfoundry.app";

        #[test]
        fn a_kept_draft_gives_its_page() {
            let body = r#"{"id": 42, "url": "https://makerfoundry.app/designs/42", "kind": "laser", "format": "SVG"}"#;
            assert_eq!(maker::read_published(SERVER, 201, body), Ok(maker::Published { id: 42, url: "https://makerfoundry.app/designs/42".into() }));
        }

        #[test]
        fn an_answer_cannot_send_the_browser_to_another_site() {
            for url in [
                "https://evil.com/designs/42",
                "http://makerfoundry.app/designs/42",
                "https://makerfoundry.app.evil.com/x",
                "https://makerfoundry.app@evil.com/x",
                "javascript:alert(1)",
                "//evil.com/x",
                "https://makerfoundry.app/a b",
                "",
            ] {
                let body = json!({"id": 1, "url": url}).to_string();
                assert_eq!(maker::read_published(SERVER, 201, &body), Err(AiError::NoAnswer), "{url:?}");
            }
            for body in [
                "",
                "not json",
                "[]",
                r#"{"id": "x", "url": "https://makerfoundry.app/d/1"}"#,
                r#"{"id": 1}"#,
                r#"{"url": "https://makerfoundry.app/d/1"}"#,
                r#"{"id": -1, "url": "https://makerfoundry.app/d/1"}"#,
            ] {
                assert_eq!(maker::read_published(SERVER, 201, body), Err(AiError::NoAnswer), "{body}");
            }
            let local = json!({"id": 1, "url": "http://127.0.0.1:8080/designs/1"}).to_string();
            assert!(maker::read_published("http://127.0.0.1:8080", 201, &local).is_ok(), "a local service answers with its own address");
            assert_eq!(maker::read_published(SERVER, 201, &local), Err(AiError::NoAnswer));
        }

        #[test]
        fn refusals_say_what_to_do() {
            let said = |m: &str| json!({"error": {"message": m}}).to_string();
            assert_eq!(maker::read_published(SERVER, 401, &said("Sign in to your Foundry Account first.")), Err(AiError::SignIn));
            assert_eq!(maker::read_published(SERVER, 413, ""), Err(AiError::TooBigToPublish));
            assert_eq!(maker::read_published(SERVER, 422, &said("That SVG has no lines.")), Err(AiError::Maker("That SVG has no lines.".into())));
            assert_eq!(maker::read_published(SERVER, 429, &said("Too many designs today.")), Err(AiError::Maker("Too many designs today.".into())));
            assert_eq!(maker::read_published(SERVER, 503, &said("Keeping designs is off.")), Err(AiError::Maker("Keeping designs is off.".into())));
            assert_eq!(maker::read_published(SERVER, 500, "oops"), Err(AiError::Service(500)));
            assert_eq!(maker::read_published(SERVER, 418, "teapot"), Err(AiError::Request(418, "teapot".into())));
            let long = maker::read_published(SERVER, 422, &said(&"x".repeat(5000)));
            assert!(matches!(long, Err(AiError::Maker(m)) if m.chars().count() == 300), "a long message is cut");
        }

        #[test]
        fn a_failed_design_fetch_is_explained_in_makerfoundrys_words() {
            let said = |m: &str| json!({"error": {"message": m}}).to_string().into_bytes();
            assert_eq!(
                maker::open_failure(404, &said("That design isn't on MakerFoundry.")),
                AiError::Maker("That design isn't on MakerFoundry.".into())
            );
            assert_eq!(maker::open_failure(403, &said("This design is for sale.")), AiError::Maker("This design is for sale.".into()));
            assert_eq!(maker::open_failure(429, b""), AiError::Maker("no details".into()));
            assert_eq!(maker::open_failure(502, b"bad gateway"), AiError::Service(502));
            assert!(matches!(maker::open_failure(404, &[0xff, 0xfe, 0x00]), AiError::Maker(_)), "a body that is not text does not break it");
            assert!(matches!(maker::open_failure(400, b"x"), AiError::Request(400, _)));
        }
    }
}
