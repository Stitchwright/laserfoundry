//! Library fonts the host has loaded into memory, for hosts with no font folder to point at (the
//! browser). A font is named `library:<folder>` (the folder it has in the font library); text commands
//! accept that name wherever they accept a font path. Fonts come from outside, so [`register`] parses
//! them with the same bounds as any other glyph font and the registry is capped.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use laserfoundry_text::GlyphFont;

use crate::EngineError;

/// The prefix of a library font's name.
pub const PREFIX: &str = "library:";
/// Most fonts held at once.
pub const MAX_FONTS: usize = 200;
/// Longest folder name accepted, in bytes.
const MAX_NAME_BYTES: usize = 80;

fn registry() -> &'static Mutex<BTreeMap<String, Arc<GlyphFont>>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, Arc<GlyphFont>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Makes the font in these two files available as `library:<folder>`, replacing any earlier one of
/// that name. `folder` is a plain name (letters, digits, `-`, `_`, `.`, space).
pub fn register(folder: &str, font_json: &str, glyphs_json: &str) -> Result<(), EngineError> {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ');
    if folder.is_empty() || folder.len() > MAX_NAME_BYTES || !folder.chars().all(plain) || folder.starts_with('.') {
        return Err(EngineError::BadParams("a library font folder name is plain letters, digits, - _ . or space".into()));
    }
    let font = GlyphFont::parse(font_json, glyphs_json).map_err(|e| EngineError::BadParams(e.to_string()))?;
    let mut fonts = registry().lock().map_err(|_| EngineError::BadParams("the font registry is unavailable".into()))?;
    let name = format!("{PREFIX}{folder}");
    if !fonts.contains_key(&name) && fonts.len() >= MAX_FONTS {
        return Err(EngineError::BadParams(format!("more than {MAX_FONTS} library fonts")));
    }
    fonts.insert(name, Arc::new(font));
    Ok(())
}

/// The names (`library:<folder>`) and display names of the fonts held, sorted by name.
pub fn list() -> Vec<(String, String)> {
    let Ok(fonts) = registry().lock() else { return Vec::new() };
    let mut out: Vec<(String, String)> =
        fonts.iter().map(|(k, f)| (k.clone(), if f.name.is_empty() { k.trim_start_matches(PREFIX).to_string() } else { f.name.clone() })).collect();
    out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Forgets every font (for tests, and a host that wants a clean slate).
pub fn clear() {
    if let Ok(mut fonts) = registry().lock() {
        fonts.clear();
    }
}

pub(crate) fn get(name: &str) -> Option<Arc<GlyphFont>> {
    registry().lock().ok()?.get(name).cloned()
}
