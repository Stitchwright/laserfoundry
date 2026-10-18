//! The desktop app's network client for the AI and account requests, and the small private file that keeps
//! an account's session token and an API key. The requests and replies are `lightfoundry-ai`'s.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use lightfoundry_ai::HttpRequest;

/// How long a request may take (the model can think for a while).
const TIMEOUT: Duration = Duration::from_secs(240);
/// The most bytes of an answer read (artwork and answers are far smaller).
const MAX_ANSWER: u64 = 16 * 1024 * 1024;
/// The most bytes the secrets file may hold.
const MAX_SECRETS: u64 = 64 * 1024;

/// Sends `request` and returns the HTTP status and body (any status: the caller reads errors). Redirects are
/// not followed, so a credential in a header never goes anywhere but where the request names.
pub fn send(request: &HttpRequest) -> Result<(u16, String), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).max_redirects(0).http_status_as_error(false).build().into();
    let mut call = agent.post(&request.url).header("User-Agent", "lightfoundry");
    for (name, value) in &request.headers {
        call = call.header(*name, value);
    }
    let response = call.send(request.body.as_bytes()).map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response.into_body().into_reader().take(MAX_ANSWER + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
    if body.len() as u64 > MAX_ANSWER {
        return Err("the answer was larger than expected".into());
    }
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

/// The value kept under `key` in the secrets file at `path`, if there is one.
pub fn secret_get(path: &Path, key: &str) -> Option<String> {
    use std::io::Read as _;
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MAX_SECRETS + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_SECRETS {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&text).ok()?.get(key)?.as_str().map(str::to_string)
}

/// Keeps `value` under `key` in the secrets file at `path`, which only its owner can read. The file is
/// replaced whole, so a crash leaves the old one.
pub fn secret_set(path: &Path, key: &str, value: &str) -> std::io::Result<()> {
    let mut map: serde_json::Map<String, serde_json::Value> =
        std::fs::read_to_string(path).ok().filter(|t| t.len() as u64 <= MAX_SECRETS).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    map.insert(key.to_string(), serde_json::Value::String(value.to_string()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let temporary = path.with_extension(format!("{}.{nanos}.tmp", std::process::id()));
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(serde_json::Value::Object(map).to_string().as_bytes())?;
        file.sync_all()
    })();
    if let Err(e) = written.and_then(|()| std::fs::rename(&temporary, path)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lf-net-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn secrets_are_kept_replaced_whole_and_private() {
        let dir = scratch("secrets");
        let file = dir.join("secrets.json");
        assert_eq!(secret_get(&file, "a"), None);
        secret_set(&file, "a", "one").unwrap();
        secret_set(&file, "b", "two").unwrap();
        secret_set(&file, "a", "three").unwrap();
        assert_eq!((secret_get(&file, "a").as_deref(), secret_get(&file, "b").as_deref()), (Some("three"), Some("two")));
        #[cfg(unix)]
        {
            let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&file).unwrap().permissions());
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(secret_get(&file, "a"), None, "a damaged file reads as empty");
        secret_set(&file, "c", "x").unwrap();
        assert_eq!(secret_get(&file, "c").as_deref(), Some("x"));
        std::fs::write(&file, vec![b' '; 70_000]).unwrap();
        assert_eq!(secret_get(&file, "c"), None, "an oversize file is ignored");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A one-shot local server: reads the request, answers with `reply`, and hands the request text back.
    fn serve_once(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if seen.len() >= end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&seen).into_owned()
        });
        (url, handle)
    }

    #[test]
    fn a_request_is_sent_with_its_headers_and_body_and_any_status_is_returned() {
        let (url, seen) = serve_once(
            "HTTP/1.1 402 Payment Required\r\ncontent-type: application/json\r\ncontent-length: 17\r\nconnection: close\r\n\r\n{\"error\":\"credit\"}",
        );
        let request = HttpRequest {
            url,
            headers: vec![("authorization", "Bearer sfs_x".into()), ("content-type", "application/json".into())],
            body: "{\"a\":1}".into(),
        };
        let (status, body) = send(&request).unwrap();
        assert_eq!(status, 402);
        assert!(body.contains("credit"));
        let text = seen.join().unwrap().to_ascii_lowercase();
        assert!(text.starts_with("post /v1/messages"), "{text}");
        assert!(text.contains("authorization: bearer sfs_x") && text.ends_with("{\"a\":1}"), "{text}");
    }

    #[test]
    fn redirects_are_not_followed_and_a_dead_server_is_an_error() {
        let (url, _seen) = serve_once("HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:1/x\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        let request = HttpRequest { url, headers: vec![], body: String::new() };
        assert_eq!(send(&request).unwrap().0, 302, "the answer is the redirect itself");
        let dead = HttpRequest { url: "http://127.0.0.1:1/".into(), headers: vec![], body: String::new() };
        assert!(send(&dead).is_err());
    }
}
