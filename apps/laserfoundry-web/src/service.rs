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
/// In a staging session (`?services=staging`) the service is the staging one and MakerFoundry's address the staging one,
/// and the real ones are refused.
pub fn post_allowed(url: &str, built_in: Option<&str>, staging: bool) -> bool {
    if staging {
        return allowed(url, Some(laserfoundry_ai::cloud::STAGING_SERVER)) || url == format!("{}/api/v1/designs", laserfoundry_ai::maker::STAGING_SERVER);
    }
    allowed(url, built_in) || url == format!("{}/api/v1/designs", laserfoundry_ai::maker::DEFAULT_SERVER)
}

/// Whether the page may GET `url`: a design file on one of MakerFoundry's own hosts (the staging one in a staging session),
/// exactly as the app checked it.
pub fn fetch_allowed(url: &str, staging: bool) -> bool {
    laserfoundry_ai::maker::open_link_in(url, staging).is_ok_and(|link| link.url == url)
}

/// The value of `services` in the page's address that switches to staging.
const STAGING: &str = "staging";

/// Whether this tab uses the staging services: the address `search` (`?services=staging&…`) says exactly `staging`, or the
/// tab remembered that it did (`remembered`, from the tab's session storage). Any other value, or none, changes nothing;
/// only these two compiled-in choices exist, so no address ever comes from the page's address.
pub fn staging_requested(search: &str, remembered: Option<&str>) -> bool {
    let asked = search.trim_start_matches('?').split('&').find_map(|pair| pair.strip_prefix("services="));
    asked == Some(STAGING) || remembered == Some(STAGING)
}

/// What the tab remembers for the rest of its session once it is in staging.
pub const REMEMBER: &str = STAGING;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makerfoundry_may_be_posted_a_design_and_asked_for_design_files_and_nothing_else() {
        let b = Some("https://cloud.example.com");
        assert!(post_allowed("https://makerfoundry.app/api/v1/designs", b, false));
        assert!(post_allowed("https://cloud.example.com/v1/messages", b, false));
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
            assert!(!post_allowed(bad, b, false), "{bad}");
        }
        assert!(fetch_allowed("https://makerfoundry.app/api/v1/designs/1/file/a.svg", false));
        assert!(fetch_allowed("https://www.makerfoundry.app/api/v1/designs/1/file/a.svg?t=x", false));
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
            assert!(!fetch_allowed(bad, false), "{bad:?}: the host fetches only what the app checked, unchanged");
        }
    }

    #[test]
    fn services_staging_switches_to_the_two_compiled_in_staging_addresses_and_nothing_else_does() {
        assert!(staging_requested("?services=staging", None));
        assert!(staging_requested("?a=1&services=staging&open=x", None));
        assert!(staging_requested("", Some("staging")), "the tab remembers it for its session");
        for unchanged in [
            "",
            "?",
            "?services=",
            "?services=production",
            "?services=Staging",
            "?services=staging2",
            "?services=staging.evil.com",
            "?services=https://evil.com",
            "?services=%73taging",
            "?xservices=staging",
            "?open=services=staging",
        ] {
            assert!(!staging_requested(unchanged, None), "{unchanged:?} must leave production in use");
            assert!(!staging_requested(unchanged, Some("production")));
        }

        let cloud = laserfoundry_ai::cloud::STAGING_SERVER;
        let maker = laserfoundry_ai::maker::STAGING_SERVER;
        assert_eq!(cloud, "https://cloud-staging-staging.up.railway.app");
        assert_eq!(maker, "https://makerfoundry-test-test.up.railway.app");
        let built_in = Some("https://cloud.example.com");
        assert!(post_allowed(&format!("{cloud}/v1/signin"), built_in, true));
        assert!(post_allowed(&format!("{maker}/api/v1/designs"), built_in, true));
        for refused in ["https://cloud.example.com/v1/signin", "https://makerfoundry.app/api/v1/designs", "https://cloud.stitchfoundry.app/v1/signin"] {
            assert!(!post_allowed(refused, built_in, true), "{refused}: staging never reaches production");
        }
        for look_alike in [
            "https://cloud-staging-staging.up.railway.app.evil.com/v1/signin",
            "https://cloud-staging-staging.up.railway.app@evil.com/v1/signin",
            "http://cloud-staging-staging.up.railway.app/v1/signin",
            "https://makerfoundry-test-test.up.railway.app.evil.com/api/v1/designs",
            "https://evil-makerfoundry-test-test.up.railway.app/api/v1/designs",
            "https://makerfoundry-test-test.up.railway.app:8443/api/v1/designs",
        ] {
            assert!(!post_allowed(look_alike, built_in, true), "{look_alike}");
        }
        // production stays the default: the staging addresses are refused unless the tab is in staging
        assert!(!post_allowed(&format!("{cloud}/v1/signin"), built_in, false));
        assert!(!post_allowed(&format!("{maker}/api/v1/designs"), built_in, false));

        let staging_file = format!("{maker}/api/v1/designs/1/file/a.svg");
        assert!(fetch_allowed(&staging_file, true));
        assert!(!fetch_allowed(&staging_file, false), "the staging host is accepted only in staging");
        assert!(!fetch_allowed("https://makerfoundry.app/api/v1/designs/1/file/a.svg", true), "and then only the staging host");
        assert!(!fetch_allowed("https://makerfoundry-test-test.up.railway.app.evil.com/api/v1/designs/1/file/a.svg", true));
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
