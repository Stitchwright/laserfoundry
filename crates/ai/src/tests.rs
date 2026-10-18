use serde_json::{Value, json};

use super::art::{self, ArtRequest, Use};
use super::check::{self, CheckRequest};
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
    fn only_https_checkout_pages_are_opened() {
        assert!(cloud::checkout_url(&json!({"url": "https://checkout.stripe.com/c/pay/x"})).is_some());
        for bad in ["http://x", "javascript:alert(1)", "file:///etc/passwd", "https://a b", ""] {
            assert_eq!(cloud::checkout_url(&json!({ "url": bad })), None, "{bad}");
        }
        assert_eq!(cloud::checkout_url(&json!({})), None);
    }
}
