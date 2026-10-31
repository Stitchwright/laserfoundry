//! MakerFoundry: sending a design to it, and opening one from it.
//!
//! Public specification: MakerFoundry's `docs/api.md` (https://github.com/SiteSentrix/makerfoundry/blob/main/docs/api.md),
//! written for this app.
//!
//! * **Publish** ([`publish`], [`read_published`]): `POST {server}/api/v1/designs` with the person's Foundry Account session
//!   token. MakerFoundry keeps the file as a draft and answers with the address of its page, where the person adds the title,
//!   licence and price: publishing always happens on the site.
//! * **Open** ([`open_link`], [`file_name`]): a MakerFoundry page links to the web app as `?open=<the design file's address>`.
//!   The address comes from outside, so it is checked strictly before anything is fetched: `https`, one of MakerFoundry's own
//!   hosts, no user name or port, nothing that could hide another host.
//!
//! Like the rest of this crate, nothing here touches the network: it builds requests and reads answers.

use serde_json::{Value, json};

use super::{AiError, HttpRequest, api_message, cloud};

/// The MakerFoundry service, used unless the person sets another (for trying one locally).
pub const DEFAULT_SERVER: &str = "https://makerfoundry.app";
/// The only hosts a design may be opened from.
pub const OPEN_HOSTS: [&str; 2] = ["makerfoundry.app", "www.makerfoundry.app"];
/// The most MakerFoundry takes for a laser file (the site's own limit).
pub const MAX_PUBLISH_BYTES: usize = 8 * 1024 * 1024;
/// The most read from a link.
pub const MAX_OPEN_BYTES: usize = 32 * 1024 * 1024;
/// The longest link and file name accepted.
const MAX_LINK: usize = 2048;
const MAX_NAME: usize = 120;
/// Where MakerFoundry takes designs from apps.
const DESIGNS: &str = "/api/v1/designs";
/// Every address a design is opened from starts like this.
const API_PREFIX: &str = "/api/v1/";

/// A design MakerFoundry kept as a draft.
#[derive(Clone, Debug, PartialEq)]
pub struct Published {
    pub id: u64,
    /// The draft's page on MakerFoundry, for the person's browser.
    pub url: String,
}

/// A link that passed the checks.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenLink {
    /// The address to fetch.
    pub url: String,
    /// The design's file name from the end of the address, when it has a usable one.
    pub name: Option<String>,
}

/// The request that sends a design: `file_name` (an `.svg` or `.dxf`), the file as `file_base64` (standard alphabet, with
/// padding) and `app` (this app and its version, for MakerFoundry's log). `token` must be a Foundry Account session token:
/// an Anthropic key is never sent here.
pub fn publish(server: &str, token: &str, file_name: &str, file_base64: &str, app: &str) -> Result<HttpRequest, AiError> {
    if !token.trim().starts_with(cloud::TOKEN_PREFIX) {
        return Err(AiError::SignIn);
    }
    let name = design_name(file_name).ok_or(AiError::NotADesign)?;
    if file_base64.is_empty() {
        return Err(AiError::NotADesign);
    }
    // base64 is four characters for every three bytes
    if file_base64.len() / 4 * 3 > MAX_PUBLISH_BYTES {
        return Err(AiError::TooBigToPublish);
    }
    let app: String = app.chars().filter(|c| !c.is_control()).take(60).collect();
    cloud::post(server, DESIGNS, Some(token), &json!({ "file_name": name, "file": file_base64, "app": app }))
}

/// What MakerFoundry answered to [`publish`]: the draft's page, or why not. A draft's page is only taken at
/// `server` itself, so an answer can't send the person's browser anywhere else.
pub fn read_published(server: &str, status: u16, body: &str) -> Result<Published, AiError> {
    match status {
        201 => {
            let v: Value = serde_json::from_str(body).map_err(|_| AiError::NoAnswer)?;
            let id = v.get("id").and_then(Value::as_u64).ok_or(AiError::NoAnswer)?;
            let url = v.get("url").and_then(Value::as_str).ok_or(AiError::NoAnswer)?;
            let same_site = cloud::clean_server(server).as_deref().and_then(origin) == origin(url);
            if !same_site || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(AiError::NoAnswer);
            }
            Ok(Published { id, url: url.to_string() })
        }
        401 | 403 => Err(AiError::SignIn),
        413 => Err(AiError::TooBigToPublish),
        400 | 422 | 429 | 503 => Err(AiError::Maker(api_message(body).chars().take(300).collect())),
        s if s >= 500 => Err(AiError::Service(s)),
        s => Err(AiError::Request(s, api_message(body))),
    }
}

/// The address from a `?open=` value (already decoded once, as the browser reads it), if it is a design file on one of
/// MakerFoundry's hosts and nothing else.
pub fn open_link(value: &str) -> Result<OpenLink, AiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_LINK || !value.is_ascii() || value.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return Err(AiError::BadLink);
    }
    let rest = value.strip_prefix("https://").ok_or(AiError::BadLink)?;
    let split = rest.find(['/', '?', '#']).ok_or(AiError::BadLink)?;
    let (host, tail) = rest.split_at(split);
    // no user name, port or IPv6 form, and the host has to be one of ours (host names have no case)
    let host = host.to_ascii_lowercase();
    if !OPEN_HOSTS.contains(&host.as_str()) {
        return Err(AiError::BadLink);
    }
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let (path, query) = tail.split_at(path_end);
    if !path.starts_with(API_PREFIX) || path.split('/').any(|part| part == "." || part == "..") {
        return Err(AiError::BadLink);
    }
    // a fragment never reaches the server; it is dropped so the address fetched is what was checked
    let query = query.split('#').next().unwrap_or_default();
    let name = path.rsplit('/').next().and_then(percent_decode).and_then(|n| design_name(&n));
    Ok(OpenLink { url: format!("https://{host}{path}{query}"), name })
}

/// The file name to open the fetched design as: the one in `Content-Disposition` when it has a usable one, else the
/// address's. It decides how the file is read, so it has to be an SVG or DXF.
pub fn file_name(link: &OpenLink, content_disposition: Option<&str>) -> Result<String, AiError> {
    content_disposition.and_then(disposition_name).or_else(|| link.name.clone()).ok_or(AiError::NotADesign)
}

/// What MakerFoundry's answer to a design fetch that wasn't a success means for the person.
pub fn open_failure(status: u16, body: &[u8]) -> AiError {
    let text = String::from_utf8_lossy(body.get(..body.len().min(4096)).unwrap_or_default());
    match status {
        403 | 404 | 429 => AiError::Maker(api_message(&text).chars().take(300).collect()),
        s if s >= 500 => AiError::Service(s),
        s => AiError::Request(s, api_message(&text)),
    }
}

/// `name` reduced to its last path part and checked: not empty, a plain name, at most [`MAX_NAME`] characters, and an `.svg`
/// or `.dxf`.
pub fn design_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next()?.trim();
    let plain = !last.is_empty() && last.chars().count() <= MAX_NAME && last != "." && last != ".." && !last.chars().any(|c| c.is_control());
    let lower = last.to_ascii_lowercase();
    (plain && (lower.ends_with(".svg") || lower.ends_with(".dxf")) && lower.len() > 4).then(|| last.to_string())
}

/// The file name in a `Content-Disposition` header (`filename*=UTF-8''…` first, then `filename="…"`).
fn disposition_name(header: &str) -> Option<String> {
    let header = header.get(..header.len().min(1000))?;
    let params: Vec<&str> = header.split(';').skip(1).map(str::trim).collect();
    let value_of = |key: &str| params.iter().find_map(|p| p.get(..key.len()).filter(|k| k.eq_ignore_ascii_case(key)).map(|_| p[key.len()..].trim()));
    if let Some(extended) = value_of("filename*=") {
        // RFC 5987: charset'language'percent-encoded; only UTF-8 is read
        let mut parts = extended.splitn(3, '\'');
        if parts.next().is_some_and(|c| c.eq_ignore_ascii_case("utf-8"))
            && let (Some(_), Some(encoded)) = (parts.next(), parts.next())
            && let Some(name) = percent_decode(encoded).and_then(|n| design_name(&n))
        {
            return Some(name);
        }
    }
    let plain = value_of("filename=")?;
    design_name(plain.trim_matches('"'))
}

/// `text` with its `%XX` escapes read, if they are all well formed and the result is UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The `https://host[:port]` part of an address.
fn origin(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let scheme = if url.starts_with("https://") { "https" } else { "http" };
    let host = rest.split(['/', '?', '#']).next().filter(|h| !h.is_empty() && !h.contains('@'))?;
    Some(format!("{scheme}://{}", host.to_ascii_lowercase()))
}
