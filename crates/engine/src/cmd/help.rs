//! Help → project links. The commands return a URL and the frontend opens it (an agent just gets the URL).

use serde_json::{Value, json};

use super::CommandSpec;
use crate::{EngineError, Session};

/// This app's source repository.
pub const GITHUB_URL: &str = "https://github.com/Stitchwright/laserfoundry";
/// Where bug reports go: StitchHub's report page, which files them as issues on this app's GitHub repository.
pub const BUG_REPORT_URL: &str = "https://hub.stitchbench.app/report-a-bug";

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "help.github",
        label: "LaserFoundry on GitHub",
        menu: "Help",
        shortcut: None,
        params_doc: "{}  — returns {url}: source code, issues and releases",
        mutates: false,
        run: github,
    },
    CommandSpec {
        id: "help.report_bug",
        label: "Report a Bug…",
        menu: "Help",
        shortcut: None,
        params_doc: "{}  — returns {url}: StitchHub's bug report page, filled in with this app, its version and the system; a report becomes an issue on GitHub",
        mutates: false,
        run: report_bug,
    },
];

fn github(_: &mut Session, _: &Value) -> Result<Value, EngineError> {
    Ok(json!({ "url": GITHUB_URL }))
}

fn report_bug(_: &mut Session, _: &Value) -> Result<Value, EngineError> {
    Ok(json!({ "url": bug_report_url(env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH) }))
}

/// The report page's address with the program, version and system in it (the page reads them; nothing else is sent).
fn bug_report_url(version: &str, os: &str, arch: &str) -> String {
    format!("{BUG_REPORT_URL}?program=laserfoundry&version={}&system={}", percent_encode(version), percent_encode(&format!("{os} {arch}")))
}

/// Percent-encodes everything but letters, digits and `-._~`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_returned_not_opened() {
        let mut s = Session::new();
        assert_eq!(s.execute("help.github", &json!({})).unwrap()["url"], GITHUB_URL);
        let u = s.execute("help.report_bug", &json!({})).unwrap()["url"].as_str().unwrap().to_string();
        assert!(u.starts_with("https://hub.stitchbench.app/report-a-bug?program=laserfoundry&version="), "{u}");
        assert!(u.contains(&format!("version={}", env!("CARGO_PKG_VERSION"))) && u.contains("&system="), "{u}");
    }

    #[test]
    fn the_address_is_encoded() {
        assert_eq!(
            bug_report_url("0.1.0 beta", "linux", "x86_64"),
            "https://hub.stitchbench.app/report-a-bug?program=laserfoundry&version=0.1.0%20beta&system=linux%20x86_64"
        );
        // Anything odd in the text can't add a parameter or leave the address.
        assert_eq!(percent_encode("a&b=c#d é"), "a%26b%3Dc%23d%20%C3%A9");
    }
}
