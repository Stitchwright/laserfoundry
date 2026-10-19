//! Which service address the hosted page may send AI and account requests to.
//!
//! The page's content security policy names one origin, set when the site is built (`LF_CLOUD_URL`); a
//! request to anywhere else would be blocked by the browser anyway, so the page refuses it first, with a
//! reason a person can read.

/// The origin (`https://host[:port]`) of `url`, if it has one.
pub fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = match url.strip_prefix("https://") {
        Some(rest) => ("https", rest),
        // plain HTTP only to this computer, for trying the service locally
        None => ("http", url.strip_prefix("http://")?),
    };
    let host = rest.split(['/', '?', '#']).next()?;
    let name = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(name, _)| name);
    let local = matches!(name, "127.0.0.1" | "localhost" | "[::1]");
    let clean = !host.is_empty() && !host.chars().any(|c| c.is_whitespace() || c.is_control() || c == '@' || c == '\\');
    (clean && (scheme == "https" || local)).then(|| format!("{scheme}://{host}"))
}

/// Whether a request to `url` may go out from a page built for the service at `built_in` (empty or absent
/// means the page was built with none).
pub fn allowed(url: &str, built_in: Option<&str>) -> bool {
    match (origin(url), built_in.and_then(origin)) {
        (Some(wanted), Some(allowed)) => wanted == allowed,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_built_in_origin_is_allowed() {
        let b = Some("https://cloud.example.com/");
        assert!(allowed("https://cloud.example.com/v1/messages", b));
        assert!(allowed("https://cloud.example.com", b));
        for bad in [
            "https://evil.example.com/v1/messages",
            "https://cloud.example.com.evil.com/x",
            "https://cloud.example.com@evil.com/x",
            "http://cloud.example.com/x",
            "http://127.0.0.1.evil.com/x",
            "https://cloud.example.com:8443/x",
            "//cloud.example.com/x",
            "https://",
            "",
        ] {
            assert!(!allowed(bad, b), "{bad}");
        }
        assert!(
            !allowed("https://cloud.example.com/x", None) && !allowed("https://cloud.example.com/x", Some("")),
            "no built-in service, none allowed"
        );
    }

    #[test]
    fn origins_are_read_strictly() {
        assert_eq!(origin("https://a.b:8443/x?y#z").as_deref(), Some("https://a.b:8443"));
        assert_eq!(origin("https://a b/x"), None);
        assert_eq!(origin("ftp://a.b"), None);
        assert_eq!(origin("http://127.0.0.1:8766/v1/x").as_deref(), Some("http://127.0.0.1:8766"));
        assert_eq!(origin("http://localhost/x").as_deref(), Some("http://localhost"));
        assert_eq!(origin("http://example.com"), None, "plain HTTP only to this computer");
    }
}
