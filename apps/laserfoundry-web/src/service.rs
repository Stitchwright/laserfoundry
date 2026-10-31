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

/// Whether the page may POST `url`: the service it was built for, or MakerFoundry's address for sending a design
/// (the page's content security policy names MakerFoundry for this and for [`fetch_allowed`]).
pub fn post_allowed(url: &str, built_in: Option<&str>) -> bool {
    allowed(url, built_in) || url == format!("{}/api/v1/designs", laserfoundry_ai::maker::DEFAULT_SERVER)
}

/// Whether the page may GET `url`: a design file on one of MakerFoundry's own hosts, exactly as the app checked it.
pub fn fetch_allowed(url: &str) -> bool {
    laserfoundry_ai::maker::open_link(url).is_ok_and(|link| link.url == url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makerfoundry_may_be_posted_a_design_and_asked_for_design_files_and_nothing_else() {
        let b = Some("https://cloud.example.com");
        assert!(post_allowed("https://makerfoundry.app/api/v1/designs", b));
        assert!(post_allowed("https://cloud.example.com/v1/messages", b));
        for bad in [
            "https://makerfoundry.app/api/v1/account",
            "https://makerfoundry.app/api/v1/designs/1",
            "https://makerfoundry.app/api/v1/designs?x=1",
            "http://makerfoundry.app/api/v1/designs",
            "https://www.makerfoundry.app/api/v1/designs",
            "https://makerfoundry.app.evil.com/api/v1/designs",
            "https://makerfoundry.app@evil.com/api/v1/designs",
            "https://evil.com/api/v1/designs",
            "",
        ] {
            assert!(!post_allowed(bad, b), "{bad}");
        }
        assert!(fetch_allowed("https://makerfoundry.app/api/v1/designs/1/file/a.svg"));
        assert!(fetch_allowed("https://www.makerfoundry.app/api/v1/designs/1/file/a.svg?t=x"));
        for bad in [
            "https://cloud.example.com/v1/account",
            "https://makerfoundry.app/login",
            "https://makerfoundry.app:8443/api/v1/designs/1/file/a.svg",
            "https://makerfoundry.app.evil.com/api/v1/designs/1/file/a.svg",
            "https://MakerFoundry.app/api/v1/designs/1/file/a.svg",
            " https://makerfoundry.app/api/v1/designs/1/file/a.svg",
            "https://makerfoundry.app/api/v1/designs/1/file/a.svg#frag",
            "",
        ] {
            assert!(!fetch_allowed(bad), "{bad:?}: the host fetches only what the app checked, unchanged");
        }
    }

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
