//! What LaserFoundry asks an AI model for, and how the answers are read.
//!
//! Four features share this crate: **Design from Prompt** and **Redraw Image** ([`art`]: vector artwork
//! for cutting and engraving), **Ask About Lasering** ([`ask`]: answers that know the open document) and
//! **Check My Cut** ([`check`]: a photo of a test cut, and the setting changes it suggests). The model is
//! Claude, reached through the Messages API either with the person's own API key or through the account
//! service ([`cloud`]), which takes credits instead.
//!
//! No network code lives here, so the browser build can use it: every function builds an [`HttpRequest`]
//! for the host to send, or reads the status and body the host got back. The StitchFoundry service speaks
//! the Anthropic Messages API (`docs/ai.md` has the endpoints).

use serde_json::{Value, json};

pub mod art;
pub mod ask;
pub mod check;
pub mod cloud;
pub mod maker;

/// The Messages API endpoint used with a person's own key.
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// The API version the requests are written against.
pub const API_VERSION: &str = "2023-06-01";
/// The model every feature asks.
pub const MODEL: &str = "claude-opus-5-5";
/// The most tokens a reply (thinking included) may use.
pub const MAX_TOKENS: u32 = 16_000;
/// The longest text a person may send as a description or question, in characters.
pub const MAX_TEXT: usize = 2_000;
/// The largest picture sent, as base64 (the API takes up to 5 MB a picture).
pub const MAX_IMAGE_BASE64: usize = 5_000_000;
/// The longest side, in pixels, a picture is worth sending at (the model sees no more detail).
pub const IMAGE_SIDE: u32 = 1568;
/// The most pictures one request carries.
pub const MAX_IMAGES: usize = 4;

/// An HTTP POST for the host's client to send.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

/// A picture to send: PNG or JPEG bytes as base64.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    /// `image/png` or `image/jpeg`.
    pub media_type: &'static str,
    pub data_base64: String,
}

impl Picture {
    pub(crate) fn block(&self) -> Value {
        json!({ "type": "image", "source": { "type": "base64", "media_type": self.media_type, "data": self.data_base64 } })
    }
}

/// How a request is paid for.
#[derive(Clone, Debug, PartialEq)]
pub enum Auth {
    /// The person's own Anthropic API key, sent straight to Anthropic.
    Key(String),
    /// A session with the account service at `server`, which takes credits.
    Session { server: String, token: String },
}

/// Why a request could not be made, or why no answer came back. The messages are for the person; the
/// interface shows them translated.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum AiError {
    #[error("Enter your Anthropic API key first.")]
    NoKey,
    #[error("The API key isn't valid: paste it again from the Anthropic Console.")]
    BadKey,
    #[error("Describe the design first.")]
    NoPrompt,
    #[error("Type a question first.")]
    NoQuestion,
    #[error("That is too long: keep it under 2,000 characters.")]
    TooLong,
    #[error("A picture is too large to send: crop it, or use a smaller copy.")]
    LargePicture,
    #[error("Add a photo of the cut first.")]
    NoPhoto,
    /// The key was refused (401) or may not use the model (403).
    #[error("Anthropic refused the API key: {0}")]
    Key(String),
    /// The account can't pay for the request (402).
    #[error("The account can't pay for this: {0}")]
    Billing(String),
    /// Too many requests, or the service is overloaded.
    #[error("The AI service is busy: try again in a minute.")]
    Busy,
    /// The service failed (5xx).
    #[error("The AI service failed (error {0}): try again later.")]
    Service(u16),
    /// Any other refusal, with the API's message.
    #[error("The request failed (error {0}): {1}")]
    Request(u16, String),
    /// The model declined to draw it.
    #[error("The AI declined to draw this: try another description.")]
    Refused,
    /// The description asks for someone else's logo or character, with the model's reason.
    #[error("The AI won't copy someone else's logo or character: {0}")]
    NotOriginal(String),
    /// The reply ran out of room before the artwork or answer was done.
    #[error("The reply grew too long to finish: ask for something simpler.")]
    Truncated,
    /// The reply holds no usable artwork.
    #[error("The reply had no artwork: try again.")]
    NoArt,
    /// The reply holds no answer.
    #[error("The reply had no answer: try again, or ask something narrower.")]
    NoAnswer,
    /// No account service is set up in this build.
    #[error("Accounts aren't open yet: set the server address in the account window.")]
    NoServer,
    /// The session ended, or there is none.
    #[error("Sign in to your account first (Help › Account…).")]
    SignIn,
    /// The account hasn't the credits.
    #[error("Not enough credits: buy more in the account window.")]
    NoCredits,
    /// An account with the email exists already.
    #[error("An account with that email already exists: sign in instead.")]
    Taken,
    /// The service can't do it (its AI or payments aren't set up).
    #[error("The service can't do this yet: try again later.")]
    CloudClosed,
    /// The request never got an answer (from the host's client).
    #[error("Couldn't reach the service: {0}")]
    Network(String),
    /// MakerFoundry refused or couldn't do it, in its own words (they are meant to be shown as they are).
    #[error("MakerFoundry says: {0}")]
    Maker(String),
    /// The design is larger than MakerFoundry takes for a laser file.
    #[error("This design is too big for MakerFoundry (the limit is 8 MB): remove some detail and try again.")]
    TooBigToPublish,
    /// A link to open a design from isn't one of MakerFoundry's.
    #[error("That link isn't one this app can open: it has to be a MakerFoundry design link.")]
    BadLink,
    /// The file behind a link isn't an SVG or DXF design.
    #[error("That file isn't an SVG or DXF design, so it can't be opened here.")]
    NotADesign,
    /// The file behind a link is larger than the app reads.
    #[error("That design is larger than this app opens from a link (32 MB).")]
    LinkTooBig,
}

impl AiError {
    /// Whether the same request may succeed if sent again shortly.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Busy | Self::Service(_) | Self::Network(_) | Self::NoArt | Self::NoAnswer)
    }
}

/// `key` checked for use in a header: trimmed, non-empty, printable ASCII without spaces.
pub fn clean_key(key: &str) -> Result<&str, AiError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(AiError::NoKey);
    }
    if key.len() > 400 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AiError::BadKey);
    }
    Ok(key)
}

/// Text trimmed and checked against [`MAX_TEXT`].
pub(crate) fn clean_text(text: &str) -> Result<String, AiError> {
    let text = text.trim();
    if text.chars().count() > MAX_TEXT {
        return Err(AiError::TooLong);
    }
    Ok(text.to_string())
}

/// The pictures checked: not too many, none too large.
pub(crate) fn check_pictures(pictures: &[Picture]) -> Result<(), AiError> {
    if pictures.len() > MAX_IMAGES || pictures.iter().any(|p| p.data_base64.len() > MAX_IMAGE_BASE64) {
        return Err(AiError::LargePicture);
    }
    Ok(())
}

/// The Messages API request body: `system` instructions and the user's `content` (text, or blocks).
pub(crate) fn body(system: &str, content: Value) -> Value {
    json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "output_config": { "effort": "medium" },
        "system": system,
        "messages": [{ "role": "user", "content": content }],
    })
}

/// `body` as an HTTP request paid for by `auth`.
pub(crate) fn send(auth: &Auth, body: &Value) -> Result<HttpRequest, AiError> {
    let json = ("content-type", "application/json".to_string());
    match auth {
        Auth::Key(key) => Ok(HttpRequest {
            url: API_URL.to_string(),
            headers: vec![json, ("x-api-key", clean_key(key)?.to_string()), ("anthropic-version", API_VERSION.to_string())],
            body: body.to_string(),
        }),
        Auth::Session { server, token } => cloud::messages(server, token, body),
    }
}

/// The API's own message in an error body (`{"type": "error", "error": {"message": …}}`), else the start
/// of the body.
pub(crate) fn api_message(body: &str) -> String {
    if let Some(m) = serde_json::from_str::<Value>(body).ok().as_ref().and_then(|v| v.pointer("/error/message")).and_then(Value::as_str) {
        return m.to_string();
    }
    let text: String = body.trim().chars().take(200).collect();
    if text.is_empty() { "no details".to_string() } else { text }
}

/// The error an HTTP status other than success stands for, for a request sent straight to Anthropic.
pub(crate) fn status_error(status: u16, body: &str) -> AiError {
    match status {
        401 | 403 => AiError::Key(api_message(body)),
        402 => AiError::Billing(api_message(body)),
        429 | 503 | 529 => AiError::Busy,
        500..=599 => AiError::Service(status),
        _ => AiError::Request(status, api_message(body)),
    }
}

/// A reply's text blocks joined (thinking left out).
pub(crate) fn reply_text(v: &Value) -> String {
    v.get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// A model's reply: its text and the model that wrote it, unless the status is an error or the model
/// stopped early. `empty` is the error for a reply with no text; `session` says the request went through
/// the account service, whose errors mean something else (sign in again, buy credits).
pub(crate) fn read_reply(status: u16, body: &str, empty: AiError, session: bool) -> Result<(String, String), AiError> {
    if !(200..300).contains(&status) {
        return Err(if session { cloud::status_error(status, body) } else { status_error(status, body) });
    }
    let v: Value = serde_json::from_str(body).map_err(|_| empty.clone())?;
    match v.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => return Err(AiError::Refused),
        Some("max_tokens") => return Err(AiError::Truncated),
        _ => {}
    }
    let text = reply_text(&v);
    if text.trim().is_empty() {
        return Err(empty);
    }
    let model = v.get("model").and_then(Value::as_str).unwrap_or(MODEL).to_string();
    Ok((text, model))
}

#[cfg(test)]
mod tests;
