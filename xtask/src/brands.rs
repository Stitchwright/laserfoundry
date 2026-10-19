//! `cargo xtask brands`: no other vendor's product or company names in what people and agents read.
//! LaserFoundry names its own features; where a comparison is needed, text says "the reference app".
//!
//! Scanned: string literals in Rust sources (labels, params docs, menus, panels, MCP tool text; comments
//! and test code are skipped), the `description` of every `Cargo.toml`, and the hosted page. Markdown docs
//! may name other software when they must (this repository's own rules do). A line that has to keep a
//! name says `brand-ok` in a comment.

use std::fs;
use std::path::Path;

/// Names that must not appear in user-visible text, lowercase, matched as whole words ignoring case.
const DENY: &[&str] = &["lightburn", "lasergrbl", "laserweb", "rdworks", "ezcad", "k40 whisperer"];
const ALLOW: &str = "brand-ok";
/// Folders with no shipped text.
const SKIP_DIRS: &[&str] = &["target", ".git", "xtask", "fuzz", "node_modules", "tests", "benches"];

#[derive(Debug, PartialEq)]
pub struct Hit {
    pub path: String,
    pub line: usize,
    pub term: &'static str,
}

pub fn run(root: &Path) -> Result<(), String> {
    let mut hits = Vec::new();
    walk(root, root, &mut hits);
    if hits.is_empty() {
        println!("brands: no vendor names in user-visible text");
        return Ok(());
    }
    let list: Vec<String> = hits.iter().map(|h| format!("{}:{}: \"{}\"", h.path, h.line, h.term)).collect();
    Err(format!("other vendors' names in user-visible text (say \"the reference app\", or mark the line `{ALLOW}`):\n  {}", list.join("\n  ")))
}

fn walk(root: &Path, dir: &Path, hits: &mut Vec<Hit>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                walk(root, &path, hits);
            }
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string();
        let is_test_file = name.starts_with("tests") || name.ends_with("_tests.rs");
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let found: Vec<(usize, &'static str)> = if name.ends_with(".rs") && !is_test_file {
            scan_rust(&text)
        } else if name == "Cargo.toml" {
            scan_descriptions(&text)
        } else if name == "index.html" {
            scan_lines(&text)
        } else {
            continue;
        };
        hits.extend(found.into_iter().map(|(line, term)| Hit { path: rel.clone(), line, term }));
    }
}

/// The denied term in `text`, if a whole word of it appears.
pub fn denied(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    DENY.iter()
        .find(|term| {
            lower.match_indices(**term).any(|(at, _)| {
                let before = lower[..at].chars().next_back();
                let after = lower[at + term.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
            })
        })
        .copied()
}

fn scan_lines(text: &str) -> Vec<(usize, &'static str)> {
    text.lines().enumerate().filter(|(_, l)| !l.contains(ALLOW)).filter_map(|(i, l)| denied(l).map(|t| (i + 1, t))).collect()
}

fn scan_descriptions(text: &str) -> Vec<(usize, &'static str)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.trim_start().starts_with("description") && !l.contains(ALLOW))
        .filter_map(|(i, l)| denied(l).map(|t| (i + 1, t)))
        .collect()
}

/// Denied names inside string literals of Rust source, skipping comments and `#[cfg(test)]` items.
pub fn scan_rust(text: &str) -> Vec<(usize, &'static str)> {
    let mut hits = Vec::new();
    let mut skip_depth: Option<i64> = None; // brace depth at which a cfg(test) item ends
    let mut depth: i64 = 0;
    let mut pending_test = false;
    for (index, line) in text.lines().enumerate() {
        let code = strip_line_comment(line);
        if code.trim_start().starts_with("#[cfg(test)]") || code.trim_start().starts_with("#[cfg(any(test") {
            pending_test = true;
        }
        let opens = code.matches('{').count() as i64;
        let closes = code.matches('}').count() as i64;
        if pending_test && opens > 0 && skip_depth.is_none() {
            skip_depth = Some(depth);
            pending_test = false;
        }
        let in_test = skip_depth.is_some() || pending_test;
        if !in_test && !line.contains(ALLOW) {
            for literal in string_literals(code) {
                if let Some(term) = denied(&literal) {
                    hits.push((index + 1, term));
                }
            }
        }
        depth += opens - closes;
        if let Some(d) = skip_depth
            && depth <= d
        {
            skip_depth = None;
        }
    }
    hits
}

/// The line without a trailing `//` comment (a `//` inside a string literal is kept).
fn strip_line_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    let bytes: Vec<(usize, char)> = line.char_indices().collect();
    for (i, (at, c)) in bytes.iter().enumerate() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
        } else if *c == '"' {
            in_string = true;
        } else if *c == '/' && bytes.get(i + 1).is_some_and(|(_, n)| *n == '/') {
            return &line[..*at];
        }
    }
    line
}

/// The contents of the `"..."` literals on a line (escapes left as written; good enough for names).
fn string_literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for c in code.chars() {
        if in_string {
            match (escaped, c) {
                (true, _) => {
                    escaped = false;
                    current.push(c);
                }
                (false, '\\') => escaped = true,
                (false, '"') => {
                    in_string = false;
                    out.push(std::mem::take(&mut current));
                }
                _ => current.push(c),
            }
        } else if c == '"' {
            in_string = true;
        }
    }
    if in_string {
        out.push(current); // a multi-line literal: check what is on this line
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    mod rust_tests {
        use super::*;

        #[test]
        fn names_in_string_literals_are_found_and_comments_and_other_words_are_not() {
            let src = "let a = \"Import from LightBurn\";\n// LightBurn compatible\nlet b = \"lightburnish\";\nlet c = \"x\"; // LightBurn\nlet d = \"a // LightBurn b\";\n";
            let hits = scan_rust(src);
            assert_eq!(hits, vec![(1, "lightburn"), (5, "lightburn")]);
        }

        #[test]
        fn test_code_and_marked_lines_are_skipped() {
            let src = "fn f() { let a = \"LightBurn\"; } // brand-ok\n#[cfg(test)]\nmod tests {\n    fn t() { let a = \"LaserGRBL\"; }\n}\nfn g() { let a = \"RDWorks\"; }\n";
            assert_eq!(scan_rust(src), vec![(6, "rdworks")]);
        }
    }

    mod other_tests {
        use super::*;

        #[test]
        fn cargo_descriptions_and_the_page_are_scanned_but_not_other_toml_lines() {
            assert_eq!(scan_descriptions("name = \"LightBurn\"\ndescription = \"a LightBurn clone\"\n"), vec![(2, "lightburn")]);
            assert_eq!(scan_lines("<title>ok</title>\n<p>EZCad</p>\n<p>EZCad <!-- brand-ok --></p>\n"), vec![(2, "ezcad")]);
            assert_eq!(denied("k40 whisperer rocks"), Some("k40 whisperer"));
            assert_eq!(denied("nothing here"), None);
        }

        #[test]
        fn the_real_repository_is_clean() {
            run(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()).unwrap();
        }
    }
}
