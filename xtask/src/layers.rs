//! Dependency layering: a crate may depend only on lower layers, and nothing below the UI layer
//! may depend on a UI toolkit. The rule engine works on a small model so it can be unit-tested.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

const PREFIX: &str = "laserfoundry-";
const UI_LAYER: u8 = 7;
const UI_TOOLKITS: &[&str] = &["egui", "eframe", "winit", "rfd", "wgpu", "glow"];

/// Packages that fetch from the network at run time. Only the desktop app and the command line may
/// use them: the hosted browser build (and the UI and engine it shares) must never download anything.
const NETWORK_CRATES: &[&str] = &["laserfoundry-fontlib", "ureq"];
const MAY_USE_NETWORK: &[&str] = &["laserfoundry", "laserfoundry-cli", "laserfoundry-fontlib", "laserfoundry-fuzzkit"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Layer(u8),
    /// Test helpers: may depend on anything below the UI; others may use them only as dev-dependencies.
    Testkit,
    /// Binaries and tooling are exempt.
    Exempt,
}

/// Package names without the `laserfoundry-` prefix.
pub const TABLE: &[(&str, Class)] = &[
    ("geom", Class::Layer(0)),
    ("i18n", Class::Layer(0)),
    ("ai", Class::Layer(0)),
    ("doc", Class::Layer(1)),
    ("pathops", Class::Layer(1)),
    ("nest", Class::Layer(1)),
    ("toolpath", Class::Layer(2)),
    ("text", Class::Layer(2)),
    ("svg", Class::Layer(3)),
    ("dxf", Class::Layer(3)),
    ("gcode", Class::Layer(3)),
    ("format", Class::Layer(3)),
    ("device", Class::Layer(3)),
    ("fontlib", Class::Layer(3)),
    ("engine", Class::Layer(5)),
    ("mcp", Class::Layer(6)),
    ("ui-egui", Class::Layer(UI_LAYER)),
    ("testkit", Class::Testkit),
    ("fuzzkit", Class::Exempt),
    ("laserfoundry", Class::Exempt),
    ("web", Class::Exempt),
    ("cli", Class::Exempt),
    ("xtask", Class::Exempt),
];

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    /// (dependency name, is dev-dependency)
    pub deps: Vec<(String, bool)>,
}

pub fn run(root: &Path) -> Result<(), String> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("could not run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let violations = check(&packages(&meta));
    if violations.is_empty() {
        println!("layers: ok");
        Ok(())
    } else {
        Err(format!("layering violations:\n  {}", violations.join("\n  ")))
    }
}

pub fn packages(meta: &Value) -> Vec<Package> {
    meta["packages"]
        .as_array()
        .map(|ps| {
            ps.iter()
                .filter_map(|p| {
                    let name = p["name"].as_str()?.to_string();
                    let deps = p["dependencies"]
                        .as_array()?
                        .iter()
                        .filter_map(|d| Some((d["name"].as_str()?.to_string(), d["kind"].as_str() == Some("dev"))))
                        .collect();
                    Some(Package { name, deps })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn class_of(name: &str) -> Option<Class> {
    let short = name.strip_prefix(PREFIX).unwrap_or(name);
    TABLE.iter().find(|(n, _)| *n == short).map(|(_, c)| *c)
}

fn level(c: Class) -> Option<u8> {
    match c {
        Class::Layer(l) => Some(l),
        Class::Testkit => Some(UI_LAYER),
        Class::Exempt => None,
    }
}

pub fn check(packages: &[Package]) -> Vec<String> {
    let mut v = Vec::new();
    for p in packages {
        for (dep, _) in &p.deps {
            if NETWORK_CRATES.contains(&dep.as_str()) && !MAY_USE_NETWORK.contains(&p.name.as_str()) {
                v.push(format!("{} depends on {dep}: only the desktop app and the CLI may download anything", p.name));
            }
        }
        let Some(class) = class_of(&p.name) else {
            v.push(format!("{} is not in the layering table (xtask/src/layers.rs)", p.name));
            continue;
        };
        let Some(my_level) = level(class) else { continue };
        for (dep, is_dev) in &p.deps {
            if my_level < UI_LAYER && UI_TOOLKITS.contains(&dep.as_str()) {
                v.push(format!("{} (layer {my_level}) depends on UI toolkit {dep}", p.name));
            }
            if !dep.starts_with(PREFIX) {
                continue;
            }
            match class_of(dep) {
                None => v.push(format!("{} depends on {dep}, which is not in the layering table", p.name)),
                Some(Class::Testkit) if !is_dev => v.push(format!("{} depends on testkit outside dev-dependencies", p.name)),
                Some(dc) => {
                    if let Some(dl) = level(dc)
                        && dl >= my_level
                        && dc != Class::Testkit
                    {
                        v.push(format!("{} (layer {my_level}) depends on {dep} (layer {dl}): dependencies must point down", p.name));
                    }
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, deps: &[(&str, bool)]) -> Package {
        Package { name: name.into(), deps: deps.iter().map(|(n, d)| ((*n).into(), *d)).collect() }
    }

    #[test]
    fn downward_dependencies_pass() {
        assert!(
            check(&[
                pkg("laserfoundry-engine", &[("laserfoundry-doc", false), ("serde", false)]),
                pkg("laserfoundry-doc", &[("laserfoundry-geom", false)])
            ])
            .is_empty()
        );
    }

    #[test]
    fn upward_and_sideways_dependencies_fail() {
        assert_eq!(check(&[pkg("laserfoundry-doc", &[("laserfoundry-engine", false)])]).len(), 1);
        assert_eq!(check(&[pkg("laserfoundry-svg", &[("laserfoundry-dxf", false)])]).len(), 1);
    }

    #[test]
    fn ui_toolkits_are_banned_below_the_ui_layer() {
        assert_eq!(check(&[pkg("laserfoundry-engine", &[("egui", false)])]).len(), 1);
        assert!(check(&[pkg("laserfoundry-ui-egui", &[("egui", false)])]).is_empty());
    }

    #[test]
    fn unknown_crates_must_be_added_to_the_table() {
        assert_eq!(check(&[pkg("laserfoundry-mystery", &[])]).len(), 1);
    }

    #[test]
    fn only_the_desktop_app_and_cli_may_download() {
        assert!(
            check(&[pkg("laserfoundry", &[("laserfoundry-fontlib", false)]), pkg("laserfoundry-cli", &[("laserfoundry-fontlib", false)])]).is_empty()
        );
        assert_eq!(check(&[pkg("laserfoundry-web", &[("laserfoundry-fontlib", false)])]).len(), 1);
        assert_eq!(check(&[pkg("laserfoundry-ui-egui", &[("ureq", false)])]).len(), 1);
        assert_eq!(check(&[pkg("laserfoundry-engine", &[("laserfoundry-fontlib", false)])]).len(), 1, "the engine runs in the browser too");
    }

    #[test]
    fn testkit_is_dev_only() {
        assert!(check(&[pkg("laserfoundry-engine", &[("laserfoundry-testkit", true)])]).is_empty());
        assert_eq!(check(&[pkg("laserfoundry-engine", &[("laserfoundry-testkit", false)])]).len(), 1);
    }

    #[test]
    fn the_real_workspace_conforms() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        run(root).unwrap();
    }
}
