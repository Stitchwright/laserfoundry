//! LaserFoundry is a clean-room project: behaviour comes from public documentation only.
//! This check fails on files or text that suggest proprietary material was copied in.

use std::fs;
use std::path::Path;

const FORBIDDEN_EXTENSIONS: &[&str] = &["lbrn", "lbrn2", "lbdev", "clb", "lblib", "lbset"];
const FORBIDDEN_TEXT: &[&str] = &["Copyright LightBurn", "(c) LightBurn", "© LightBurn", "LightBurn Software, LLC"];
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules"];
/// This file and the docs that state the rule necessarily mention the forbidden strings.
const ALLOWED_FILES: &[&str] = &["cleanroom.rs", "AGENTS.md", "CLAUDE.md"];

pub fn run(root: &Path) -> Result<(), String> {
    let mut problems = Vec::new();
    walk(root, root, &mut problems);
    if problems.is_empty() {
        println!("cleanroom: ok");
        Ok(())
    } else {
        Err(format!("clean-room violations:\n  {}", problems.join("\n  ")))
    }
}

fn walk(root: &Path, dir: &Path, problems: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &path, problems);
            }
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if FORBIDDEN_EXTENSIONS.contains(&ext.as_str()) {
            problems.push(format!("{rel}: LightBurn project/device/library files must not be committed"));
            continue;
        }
        if ALLOWED_FILES.contains(&name.as_str()) {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            for needle in FORBIDDEN_TEXT {
                if text.contains(needle) {
                    problems.push(format!("{rel}: contains {needle:?}"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_forbidden_files_and_text_but_not_clean_trees() {
        let dir = std::env::temp_dir().join(format!("lf-cleanroom-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ok.txt"), "nothing to see").unwrap();
        assert!(run(&dir).is_ok());
        fs::write(dir.join("sample.lbrn2"), "x").unwrap();
        fs::write(dir.join("notice.txt"), "Copyright LightBurn 2024").unwrap();
        let err = run(&dir).unwrap_err();
        assert!(err.contains("sample.lbrn2") && err.contains("notice.txt"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }
}
