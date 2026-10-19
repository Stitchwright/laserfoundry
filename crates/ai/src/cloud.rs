//! The account service: sign up and in, read the credits, buy more, and send AI requests that it pays for
//! with credits (`docs/ai.md`). One service answers every app that uses it, so an account and its credits
//! are shared. Everything here builds a request or reads an answer; the host does the sending.

use serde_json::{Value, json};

use super::{AiError, HttpRequest, api_message};

/// Session tokens start with this.
pub const TOKEN_PREFIX: &str = "sfs_";
/// The longest session token accepted.
const MAX_TOKEN: usize = 200;

/// The service's endpoint for AI requests.
pub const MESSAGES: &str = "/v1/messages";
/// The shared Foundry account service, used when no other address is set at build time.
pub const DEFAULT_SERVER: &str = "https://cloud.stitchfoundry.app";

/// The service this build talks to: `LF_CLOUD_URL` when the program was built, else the shared Foundry service.
pub const fn built_in_server() -> &'static str {
    match option_env!("LF_CLOUD_URL") {
        Some(url) if !url.is_empty() => url,
        _ => DEFAULT_SERVER,
    }
}

/// `server` tidied (no trailing slash) when it is an address the app may send to: HTTPS, or plain HTTP on
/// this computer (for running the service locally).
pub fn clean_server(server: &str) -> Option<String> {
    let s = server.trim().trim_end_matches('/');
    let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| s.strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with(':') || rest.starts_with('/')));
    let secure = s.strip_prefix("https://").is_some_and(|rest| !rest.is_empty());
    ((secure || local) && !s.chars().any(|c| c.is_whitespace() || c.is_control())).then(|| s.to_string())
}

/// A POST of `body` to `path` on `server`, signed in with `token` when given.
fn post(server: &str, path: &str, token: Option<&str>, body: &Value) -> Result<HttpRequest, AiError> {
    let server = clean_server(server).ok_or(AiError::NoServer)?;
    let mut headers = vec![("content-type", "application/json".to_string())];
    if let Some(token) = token {
        let token = token.trim();
        if token.is_empty() || token.len() > MAX_TOKEN || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(AiError::SignIn);
        }
        headers.push(("authorization", format!("Bearer {token}")));
    }
    Ok(HttpRequest { url: format!("{server}{path}"), headers, body: body.to_string() })
}

pub fn sign_up(server: &str, email: &str, password: &str) -> Result<HttpRequest, AiError> {
    post(server, "/v1/signup", None, &json!({ "email": email.trim(), "password": password }))
}

pub fn sign_in(server: &str, email: &str, password: &str) -> Result<HttpRequest, AiError> {
    post(server, "/v1/signin", None, &json!({ "email": email.trim(), "password": password }))
}

pub fn sign_out(server: &str, token: &str) -> Result<HttpRequest, AiError> {
    post(server, "/v1/signout", Some(token), &json!({}))
}

/// The account: `{email, credits, packs: [{id, credits, cents}], payments, history}`.
pub fn account(server: &str, token: &str) -> Result<HttpRequest, AiError> {
    post(server, "/v1/account", Some(token), &json!({}))
}

/// A checkout page for credit pack `pack`: the answer's `url` is for the person's browser.
pub fn checkout(server: &str, token: &str, pack: &str) -> Result<HttpRequest, AiError> {
    post(server, "/v1/checkout", Some(token), &json!({ "pack": pack }))
}

/// An AI request body sent to the service, paid for in credits.
pub fn messages(server: &str, token: &str, body: &Value) -> Result<HttpRequest, AiError> {
    if !token.trim().starts_with(TOKEN_PREFIX) {
        return Err(AiError::SignIn); // an Anthropic key must never be sent to the service
    }
    post(server, MESSAGES, Some(token), body)
}

/// The error an answer from the service with HTTP status `status` and body `body` means.
pub fn status_error(status: u16, body: &str) -> AiError {
    match status {
        401 | 403 => AiError::SignIn,
        402 => AiError::NoCredits,
        409 => AiError::Taken,
        429 | 529 => AiError::Busy,
        503 => AiError::CloudClosed,
        s if s >= 500 => AiError::Service(s),
        s => AiError::Request(s, api_message(body)),
    }
}

/// An answer from the service: its JSON, or what went wrong.
pub fn parse(status: u16, body: &str) -> Result<Value, AiError> {
    if !(200..300).contains(&status) {
        return Err(status_error(status, body));
    }
    serde_json::from_str(body).map_err(|_| AiError::NoAnswer)
}

/// What a sign-in or sign-up answered: the session token and the account's email, if they are there.
pub fn session(answer: &Value) -> Option<(String, String)> {
    let token = answer.get("token")?.as_str()?;
    let email = answer.pointer("/account/email").and_then(Value::as_str).unwrap_or_default();
    (token.starts_with(TOKEN_PREFIX) && token.len() <= MAX_TOKEN).then(|| (token.to_string(), email.chars().take(200).collect()))
}

/// The account's figures as shown in the window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub email: String,
    pub credits: u64,
    pub packs: Vec<Pack>,
    /// Whether the service can take payments.
    pub payments: bool,
}

/// A bundle of credits on sale.
#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    pub id: String,
    pub credits: u64,
    pub cents: u64,
}

/// The most packs read from an answer.
const MAX_PACKS: usize = 20;

/// The account in an `account` answer (anything missing or odd is left out, never an error).
pub fn read_account(answer: &Value) -> Account {
    let number = |v: &Value| v.as_u64().or_else(|| v.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64)).unwrap_or(0);
    let packs = answer
        .get("packs")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_PACKS)
                .filter_map(|p| {
                    let id = p.get("id")?.as_str()?;
                    (!id.is_empty() && id.len() <= 40).then(|| Pack {
                        id: id.to_string(),
                        credits: number(&p["credits"]),
                        cents: number(&p["cents"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Account {
        email: answer.get("email").and_then(Value::as_str).unwrap_or_default().chars().take(200).collect(),
        credits: number(&answer["credits"]),
        packs,
        payments: answer.get("payments").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// The page to open for a purchase, from a `checkout` answer: HTTPS only.
pub fn checkout_url(answer: &Value) -> Option<String> {
    let url = answer.get("url")?.as_str()?;
    (url.starts_with("https://") && url.len() <= 2000 && !url.chars().any(|c| c.is_whitespace() || c.is_control())).then(|| url.to_string())
}

/// A price in cents as dollars, for the window (`1000` → `$10.00`).
pub fn dollars(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}
