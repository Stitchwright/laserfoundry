//! "Never break wasm": the browser build compiles these crates for `wasm32-unknown-unknown`, so every
//! change is checked against that target. Each workspace package is either in [`WASM`] (checked) or in
//! [`NATIVE_ONLY`] (network, sockets, serial ports, binaries and tooling); a new package must be
//! classified, so none is forgotten.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

const TARGET: &str = "wasm32-unknown-unknown";
const PREFIX: &str = "laserfoundry-";

/// Package names (without the `laserfoundry-` prefix) the browser build contains.
pub const WASM: &[&str] =
    &["geom", "doc", "pathops", "nest", "toolpath", "text", "svg", "dxf", "gcode", "format", "device", "i18n", "ai", "engine", "ui-egui", "web"];

/// Packages that never run in the browser, and why.
pub const NATIVE_ONLY: &[&str] = &[
    "mcp",          // the agent link listens on a local socket
    "icon",         // desktop packaging only: draws the application icon files
    "fontlib",      // downloads over the network
    "fuzzkit",      // test drivers
    "testkit",      // test helpers
    "laserfoundry", // the desktop app
    "cli",          // the command line
    "xtask",        // this tool
];

pub fn run(root: &Path) -> Result<(), String> {
    let names = workspace_packages(root)?;
    let unclassified = unclassified(&names);
    if !unclassified.is_empty() {
        return Err(format!("not classified for the wasm check (add to xtask/src/wasm.rs): {}", unclassified.join(", ")));
    }
    let installed =
        Command::new("rustup").args(["target", "list", "--installed"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    if installed.is_some_and(|t| !t.lines().any(|l| l.trim() == TARGET)) {
        return Err(format!("the {TARGET} target is not installed: rustup target add {TARGET}"));
    }
    let mut failed = Vec::new();
    for short in WASM {
        let package = package_name(short);
        eprintln!("==> wasm: {package}");
        let status = Command::new("cargo")
            .args(["check", "--target", TARGET, "-p", &package])
            .current_dir(root)
            .status()
            .map_err(|e| format!("could not run cargo: {e}"))?;
        if !status.success() {
            failed.push(package);
        }
    }
    if failed.is_empty() {
        println!("wasm: ok ({} crates check for {TARGET})", WASM.len());
        Ok(())
    } else {
        Err(format!("these crates no longer build for {TARGET}: {}", failed.join(", ")))
    }
}

/// `geom` is `laserfoundry-geom`; the desktop app and xtask carry no prefix of their own.
fn package_name(short: &str) -> String {
    if short == "laserfoundry" || short == "xtask" { short.to_string() } else { format!("{PREFIX}{short}") }
}

fn short_name(package: &str) -> &str {
    package.strip_prefix(PREFIX).unwrap_or(package)
}

/// Workspace packages in neither list.
pub fn unclassified(names: &[String]) -> Vec<String> {
    names.iter().filter(|n| !WASM.contains(&short_name(n)) && !NATIVE_ONLY.contains(&short_name(n))).cloned().collect()
}

fn workspace_packages(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("could not run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    Ok(meta["packages"].as_array().map(|ps| ps.iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_package_must_be_classified() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(unclassified(&names(&["laserfoundry-geom", "laserfoundry-mcp", "xtask", "laserfoundry"])).is_empty());
        assert_eq!(unclassified(&names(&["laserfoundry-mystery"])), ["laserfoundry-mystery"]);
    }

    #[test]
    fn every_real_workspace_package_is_classified_and_the_lists_do_not_overlap() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let names = workspace_packages(root).unwrap();
        assert_eq!(unclassified(&names), Vec::<String>::new());
        assert!(WASM.iter().all(|w| !NATIVE_ONLY.contains(w)));
        assert!(WASM.iter().all(|w| names.contains(&package_name(w))), "every listed crate exists");
    }
}
