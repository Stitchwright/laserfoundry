//! Translating the interface. Every fixed piece of text is passed through [`tr`], which looks the
//! English text up in a catalog the person supplies and returns it unchanged when there is no
//! catalog or no entry. No translation ships with the program; a catalog is a JSON file:
//!
//! ```json
//! { "name": "Deutsch", "font": "/path/to/a/font.ttf", "strings": { "Open…": "Öffnen…" } }
//! ```
//!
//! `font` is optional: egui's built-in font covers Latin scripts only, so a language written in
//! another script needs a font file that has it (the program ships none). "Save translation template"
//! in the Help menu writes every string seen so far with itself as the value, to start from.
//! Engine error messages are translated by template with [`tr_message`].

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Largest catalog file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Most entries accepted.
pub const MAX_STRINGS: usize = 20_000;
/// Longest key or translation accepted, in characters.
pub const MAX_LEN: usize = 2_000;
/// Largest font file read, in bytes.
pub const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Translation {
    pub name: String,
    pub font: Option<PathBuf>,
    pub strings: HashMap<String, String>,
    /// Entries whose English text has `{name}` places, for matching messages that carry values.
    patterns: Vec<Pattern>,
}

/// A message template such as `shape {id} does not exist`, split into fixed text and named places.
#[derive(Debug, Clone, PartialEq)]
struct Pattern {
    /// Fixed text before each place, then the text after the last one (`names.len() + 1` pieces).
    literals: Vec<String>,
    names: Vec<String>,
    value: String,
}

impl Pattern {
    /// `None` when the key has no places. A bare `{}` is named by its position (`1`, `2`, …); a
    /// format spec after a colon (`{n:?}`) is ignored.
    fn new(key: &str, value: &str) -> Option<Pattern> {
        let (mut literals, mut names) = (vec![String::new()], Vec::new());
        let mut rest = key;
        while let Some(open) = rest.find(['{', '}']) {
            let (before, tail) = rest.split_at(open);
            let current = literals.last_mut()?;
            current.push_str(before);
            if tail.starts_with("{{") || tail.starts_with("}}") {
                current.push_str(&tail[..1]);
                rest = &tail[2..];
                continue;
            }
            let Some(close) = tail.strip_prefix('{').and_then(|t| t.find('}')) else {
                current.push_str(&tail[..1]);
                rest = &tail[1..];
                continue;
            };
            let token = &tail[1..=close];
            let name = token.split(':').next().unwrap_or("");
            names.push(if name.is_empty() { (names.len() + 1).to_string() } else { name.to_string() });
            literals.push(String::new());
            rest = &tail[close + 2..];
        }
        literals.last_mut()?.push_str(rest);
        (!names.is_empty()).then(|| Pattern { literals, names, value: value.to_string() })
    }

    /// The text each place stands for, when `message` has this shape.
    fn captures<'a>(&self, message: &'a str) -> Option<Vec<&'a str>> {
        let first = self.literals.first()?;
        let last = self.literals.last()?;
        let mut rest = message.strip_prefix(first.as_str())?;
        rest = rest.strip_suffix(last.as_str())?;
        let mut found = Vec::with_capacity(self.names.len());
        for literal in self.literals.iter().skip(1).take(self.names.len().saturating_sub(1)) {
            let at = if literal.is_empty() { 0 } else { rest.find(literal.as_str())? };
            found.push(&rest[..at]);
            rest = &rest[at + literal.len()..];
        }
        found.push(rest);
        (found.len() == self.names.len()).then_some(found)
    }

    fn fixed_len(&self) -> usize {
        self.literals.iter().map(String::len).sum()
    }
}

// The interface runs on one thread, so the catalog is per thread: nothing else can change it under a
// frame being drawn, and tests that load different catalogs cannot disturb one another.
thread_local! {
    static ACTIVE: RefCell<Option<Translation>> = const { RefCell::new(None) };
    static SEEN: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// The text to show for `english`: its translation if the active catalog has one.
pub fn tr(english: &str) -> String {
    SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        if !seen.contains(english) {
            seen.insert(english.to_string());
        }
    });
    ACTIVE.with(|a| a.borrow().as_ref().and_then(|t| t.strings.get(english)).map_or_else(|| english.to_string(), Clone::clone))
}

/// Translates a message the program received rather than wrote, such as an engine error: the whole
/// text if the catalog has it, else the template it matches (`"shape {id} does not exist"`), with the
/// values filled in and themselves translated (an error that wraps another one translates both).
/// Nothing is recorded for the template; the list of engine messages ships with the program.
pub fn tr_message(message: &str) -> String {
    message_at(message, 0)
}

fn message_at(message: &str, depth: usize) -> String {
    ACTIVE.with(|a| a.borrow().as_ref().map_or_else(|| message.to_string(), |t| t.message(message, depth)))
}

impl Translation {
    /// [`tr_message`] against this catalog alone.
    pub fn translate_message(&self, message: &str) -> String {
        self.message(message, 0)
    }

    fn message(&self, message: &str, depth: usize) -> String {
        if let Some(v) = self.strings.get(message) {
            return v.clone();
        }
        if depth >= 3 {
            return message.to_string();
        }
        for p in &self.patterns {
            if let Some(caps) = p.captures(message) {
                let args: Vec<(&str, String)> = p.names.iter().map(String::as_str).zip(caps.iter().map(|c| self.message(c, depth + 1))).collect();
                return fill(&p.value, &args);
            }
        }
        message.to_string()
    }
}

/// Every message template the program's own errors can use, for translators (a list kept up to
/// date by `cargo xtask messages`).
pub const ENGINE_MESSAGES: &str = include_str!("../engine-messages.txt");

/// [`tr`] for a text with values in it: the template has `{name}` places, filled from `args`. The
/// values are inserted as they are (a value that itself contains `{name}` is not filled again), and
/// a place with no matching argument is left in the text.
pub fn tr_fill(template: &str, args: &[(&str, String)]) -> String {
    fill(&tr(template), args)
}

fn fill(text: &str, args: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}').map(|close| (&after[..close], &after[close + 1..])) {
            Some((name, tail)) if args.iter().any(|(k, _)| *k == name) => {
                if let Some((_, value)) = args.iter().find(|(k, _)| *k == name) {
                    out.push_str(value);
                }
                rest = tail;
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `trf!("Opened {name}", name = file)`: a translated text with values filled in (see [`tr_fill`]).
#[macro_export]
macro_rules! trf {
    ($template:literal $(, $key:ident = $value:expr)* $(,)?) => {
        $crate::tr_fill($template, &[$((stringify!($key), ($value).to_string())),*])
    };
}

/// Makes `translation` the one in use (`None` goes back to English).
pub fn set_active(translation: Option<Translation>) {
    ACTIVE.with(|a| *a.borrow_mut() = translation);
}

/// The font file the active catalog asks for, if any.
pub fn active_font() -> Option<PathBuf> {
    ACTIVE.with(|a| a.borrow().as_ref().and_then(|t| t.font.clone()))
}

/// The name of the active catalog, if there is one.
pub fn active_name() -> Option<String> {
    ACTIVE.with(|a| a.borrow().as_ref().map(|t| t.name.clone()))
}

/// Reads a font file for the active catalog, refusing one that is too large.
pub fn read_font(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| trf!("cannot open the font {path}: {e}", path = path.display(), e = e))?;
    let mut data = Vec::new();
    file.take(MAX_FONT_BYTES + 1).read_to_end(&mut data).map_err(|e| trf!("cannot read the font {path}: {e}", path = path.display(), e = e))?;
    if data.len() as u64 > MAX_FONT_BYTES {
        return Err(tr("the font file is too large"));
    }
    Ok(data)
}

/// Every string asked for so far, as a catalog with each string as its own translation.
pub fn template() -> String {
    let seen = SEEN.with(|s| s.borrow().clone());
    let strings: serde_json::Map<String, serde_json::Value> = seen
        .iter()
        .map(String::as_str)
        .chain(ENGINE_MESSAGES.lines().filter(|l| !l.is_empty()))
        .map(|s| (s.to_string(), serde_json::Value::String(s.to_string())))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "name": "", "font": "", "strings": strings })).unwrap_or_default()
}

/// Parses a catalog. Unusable input is an error with a reason; nothing here can panic.
pub fn parse(text: &str) -> Result<Translation, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| trf!("not a translation file: {e}", e = e))?;
    let obj = v.as_object().ok_or_else(|| tr("a translation file is a JSON object"))?;
    let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("").chars().take(80).collect();
    let font = obj.get("font").and_then(|f| f.as_str()).filter(|f| !f.is_empty()).map(PathBuf::from);
    let raw = obj.get("strings").and_then(|s| s.as_object()).ok_or_else(|| tr("the file has no \"strings\" object"))?;
    if raw.len() > MAX_STRINGS {
        return Err(trf!("more than {max} strings", max = MAX_STRINGS));
    }
    let mut strings = HashMap::with_capacity(raw.len());
    for (key, value) in raw {
        let Some(value) = value.as_str() else { return Err(trf!("the translation of {key} is not text", key = format!("{key:?}"))) };
        if key.chars().count() > MAX_LEN || value.chars().count() > MAX_LEN {
            return Err(tr("a string is too long"));
        }
        if !value.is_empty() && value != key {
            strings.insert(key.clone(), value.to_string());
        }
    }
    let mut patterns: Vec<Pattern> = strings.iter().filter_map(|(k, v)| Pattern::new(k, v)).collect();
    // The more fixed text a template has, the more specific it is: try those first.
    patterns.sort_by(|a, b| b.fixed_len().cmp(&a.fixed_len()).then_with(|| a.literals.cmp(&b.literals)));
    Ok(Translation { name, font, strings, patterns })
}

/// Reads and parses the catalog at `path`.
pub fn load(path: &Path) -> Result<Translation, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| trf!("cannot open {path}: {e}", path = path.display(), e = e))?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text).map_err(|e| trf!("cannot read {path}: {e}", path = path.display(), e = e))?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(tr("the translation file is too large"));
    }
    parse(&text)
}

/// Where the person's catalog is kept: `translation.json` in the per-user data folder
/// (`$XDG_DATA_HOME` or `~/.local/share` on Linux, `~/Library/Application Support` on macOS,
/// `%APPDATA%` on Windows) under `LaserFoundry`.
pub fn default_file() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
    };
    Some(base?.join("LaserFoundry").join("translation.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalog_parses_and_translates_with_english_as_the_fallback() {
        let t = parse(r#"{"name":"Test","strings":{"Open…":"Öffnen…","Same":"Same","Empty":""}}"#).unwrap();
        assert_eq!(t.name, "Test");
        assert_eq!(t.strings.len(), 1, "identical and empty entries are not kept");
        assert_eq!(t.strings.get("Open…").map(String::as_str), Some("Öffnen…"));
    }

    #[test]
    fn catalog_errors_follow_the_active_catalog() {
        assert_eq!(parse("[]").err().as_deref(), Some("a translation file is a JSON object"));
        set_active(parse(r#"{"strings":{"a translation file is a JSON object":"ein Katalog ist ein JSON-Objekt"}}"#).ok());
        let err = parse("[]").err();
        set_active(None);
        assert_eq!(err.as_deref(), Some("ein Katalog ist ein JSON-Objekt"));
    }

    #[test]
    fn values_are_filled_into_translated_templates_without_being_filled_again() {
        assert_eq!(tr_fill("Job {n} of {total}", &[("n", "3".into()), ("total", "9".into())]), "Job 3 of 9");
        assert_eq!(tr_fill("{a}{b}", &[("a", "{b}".into()), ("b", "x".into())]), "{b}x", "values are not re-expanded");
        assert_eq!(tr_fill("odd { brace and {missing}", &[("n", "1".into())]), "odd { brace and {missing}");
        assert_eq!(tr_fill("", &[]), "");
        let t = parse(r#"{"strings":{"i18n-fill {n} items":"{n} Einträge (i18n-fill)"}}"#).unwrap();
        set_active(Some(t));
        assert_eq!(trf!("i18n-fill {n} items", n = 4), "4 Einträge (i18n-fill)");
        set_active(None);
        assert_eq!(trf!("i18n-fill {n} items", n = 4), "i18n-fill 4 items");
    }

    #[test]
    fn engine_messages_are_matched_by_template_and_nested_ones_are_translated_too() {
        let t = parse(
            r#"{"strings":{
                "shape {id} does not exist":"Form {id} gibt es nicht",
                "layer {} does not exist":"Ebene {1} gibt es nicht",
                "bad parameters: {0}":"falsche Angaben: {0}",
                "{name} is not text":"{name} ist kein Text",
                "nothing is selected":"nichts ausgewählt",
                "{a} and {b}":"{b} und {a}"
            }}"#,
        )
        .unwrap();
        set_active(Some(t));
        assert_eq!(tr_message("shape 12 does not exist"), "Form 12 gibt es nicht");
        assert_eq!(tr_message("layer 3 does not exist"), "Ebene 3 gibt es nicht");
        assert_eq!(tr_message("nothing is selected"), "nichts ausgewählt");
        assert_eq!(tr_message("bad parameters: shape 7 does not exist"), "falsche Angaben: Form 7 gibt es nicht");
        assert_eq!(tr_message("bad parameters: Box 1 is not text"), "falsche Angaben: Box 1 ist kein Text");
        assert_eq!(tr_message("x and y"), "y und x", "places can swap");
        assert_eq!(tr_message("something nobody translated"), "something nobody translated");
        assert_eq!(tr_message(""), "");
        assert_eq!(tr_message("shape  does not exist"), "Form  gibt es nicht", "an empty value still matches");
        set_active(None);
        assert_eq!(tr_message("shape 12 does not exist"), "shape 12 does not exist");
    }

    #[test]
    fn the_shipped_message_list_is_one_template_per_line() {
        assert!(ENGINE_MESSAGES.lines().count() > 20);
        assert!(ENGINE_MESSAGES.lines().all(|l| !l.trim().is_empty() && l == l.trim()));
    }

    #[test]
    fn hostile_catalogs_are_errors() {
        for text in ["", "nope", "[]", "{}", r#"{"strings":[]}"#, r#"{"strings":{"a":1}}"#] {
            assert!(parse(text).is_err(), "{text}");
        }
        let long = format!(r#"{{"strings":{{"a":"{}"}}}}"#, "x".repeat(MAX_LEN + 1));
        assert!(parse(&long).is_err());
        let many: String = (0..=MAX_STRINGS).map(|i| format!("\"k{i}\":\"v\"")).collect::<Vec<_>>().join(",");
        assert!(parse(&format!(r#"{{"strings":{{{many}}}}}"#)).is_err());
    }

    #[test]
    fn files_are_read_with_limits_and_the_template_lists_what_was_asked_for() {
        let dir = std::env::temp_dir().join(format!("lf-i18n-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.json");
        std::fs::write(&file, r#"{"strings":{"i18n-test-key":"Übersetzt"}}"#).unwrap();
        let t = load(&file).unwrap();
        set_active(Some(t));
        assert_eq!(tr("i18n-test-key"), "Übersetzt");
        assert_eq!(tr("i18n-other-key"), "i18n-other-key");
        let template: serde_json::Value = serde_json::from_str(&template()).unwrap();
        assert!(template["strings"]["i18n-test-key"].is_string() && template["strings"]["i18n-other-key"].is_string());
        set_active(None);
        assert_eq!(tr("i18n-test-key"), "i18n-test-key");
        assert!(load(&dir.join("missing.json")).is_err());
        std::fs::write(&file, vec![b' '; MAX_FILE_BYTES as usize + 10]).unwrap();
        assert!(load(&file).is_err());
        assert!(read_font(&dir.join("missing.ttf")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
