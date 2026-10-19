//! The hosted site's font index (`fonts/index.json`, written by `cargo xtask site-fonts`): which fonts the
//! site carries and the files each needs. Pure and bounded, so it can be tested without a browser: the
//! index is fetched over the network and is untrusted.

use serde_json::Value;

/// Most fonts accepted from an index.
pub const MAX_FONTS: usize = 100;
/// Largest index text read, in bytes.
pub const MAX_INDEX_BYTES: usize = 512 * 1024;
/// Largest single font file, matching what the engine will parse.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The font's folder on the site (a plain name).
    pub folder: String,
    pub name: String,
    pub font_json_bytes: u64,
    pub glyphs_bytes: u64,
}

/// The fonts an index lists that have both files and plain names; anything else is skipped.
pub fn parse_index(text: &str) -> Result<Vec<Entry>, String> {
    if text.len() > MAX_INDEX_BYTES {
        return Err("the font index is too large".into());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("the font index is not valid: {e}"))?;
    let list = v.get("fonts").and_then(Value::as_array).ok_or("the font index has no \"fonts\" list")?;
    let mut out = Vec::new();
    for item in list.iter().take(MAX_FONTS) {
        let Some(folder) = item.get("folder").and_then(Value::as_str).filter(|f| plain(f)) else { continue };
        let size_of = |file: &str| {
            item.get("files")?
                .as_array()?
                .iter()
                .find(|f| f.get("path").and_then(Value::as_str) == Some(file))?
                .get("size")?
                .as_u64()
                .filter(|s| *s <= MAX_FILE_BYTES)
        };
        let (Some(font_json_bytes), Some(glyphs_bytes)) = (size_of("font.json"), size_of("glyphs.stitchfoundry")) else { continue };
        let name = item.get("name").and_then(Value::as_str).unwrap_or(folder).chars().take(80).collect();
        out.push(Entry { folder: folder.to_string(), name, font_json_bytes, glyphs_bytes });
    }
    Ok(out)
}

/// A folder name that is safe to put in a URL path and to register.
pub fn plain(name: &str) -> bool {
    !name.is_empty() && name.len() <= 80 && !name.starts_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The folders remembered from an earlier visit (a JSON list of names); damaged text gives none.
pub fn parse_remembered(text: &str) -> Vec<String> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    list.iter().filter_map(Value::as_str).filter(|f| plain(f)).take(MAX_FONTS).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    mod index_tests {
        use super::*;

        fn index(entries: &str) -> String {
            format!(r#"{{"fonts":[{entries}]}}"#)
        }

        #[test]
        fn complete_plain_entries_are_listed() {
            let text = index(
                r#"{"folder":"foundry-block","name":"Foundry Block","files":[{"path":"font.json","size":10},{"path":"glyphs.stitchfoundry","size":2000},{"path":"OFL.txt","size":5}]},
                   {"folder":"nameless","files":[{"path":"font.json","size":1},{"path":"glyphs.stitchfoundry","size":2}]}"#,
            );
            let fonts = parse_index(&text).unwrap();
            assert_eq!(fonts.len(), 2);
            assert_eq!(fonts[0], Entry { folder: "foundry-block".into(), name: "Foundry Block".into(), font_json_bytes: 10, glyphs_bytes: 2000 });
            assert_eq!(fonts[1].name, "nameless", "the folder stands in for a missing name");
        }

        #[test]
        fn incomplete_unsafe_or_oversized_entries_are_skipped() {
            let text = index(
                r#"{"folder":"no-glyphs","files":[{"path":"font.json","size":1}]},
                   {"folder":"../up","files":[{"path":"font.json","size":1},{"path":"glyphs.stitchfoundry","size":1}]},
                   {"folder":"a/b","files":[{"path":"font.json","size":1},{"path":"glyphs.stitchfoundry","size":1}]},
                   {"folder":".hidden","files":[{"path":"font.json","size":1},{"path":"glyphs.stitchfoundry","size":1}]},
                   {"folder":"huge","files":[{"path":"font.json","size":1},{"path":"glyphs.stitchfoundry","size":99999999999}]},
                   {"folder":7},
                   {"folder":"ok","files":[{"path":"font.json","size":1},{"path":"glyphs.stitchfoundry","size":2}]}"#,
            );
            let fonts = parse_index(&text).unwrap();
            assert_eq!(fonts.iter().map(|f| f.folder.as_str()).collect::<Vec<_>>(), ["ok"]);
        }

        #[test]
        fn a_damaged_index_is_an_error_and_a_flood_is_capped() {
            for text in ["", "not json", "[]", "{}", r#"{"fonts":3}"#] {
                assert!(parse_index(text).is_err(), "{text:?}");
            }
            assert!(parse_index(&" ".repeat(MAX_INDEX_BYTES + 1)).is_err());
            let many: Vec<String> = (0..500)
                .map(|i| format!(r#"{{"folder":"f{i}","files":[{{"path":"font.json","size":1}},{{"path":"glyphs.stitchfoundry","size":1}}]}}"#))
                .collect();
            assert_eq!(parse_index(&index(&many.join(","))).unwrap().len(), MAX_FONTS);
        }
    }

    mod remembered_tests {
        use super::*;

        #[test]
        fn remembered_folders_are_read_back_and_damage_gives_none() {
            assert_eq!(parse_remembered(r#"["foundry-block","../x","a b",5,"ok_2"]"#), ["foundry-block", "ok_2"]);
            for text in ["", "{}", "null", "not json", "[[1]]"] {
                assert!(parse_remembered(text).is_empty(), "{text:?}");
            }
            let many = format!("[{}]", (0..500).map(|i| format!("\"f{i}\"")).collect::<Vec<_>>().join(","));
            assert_eq!(parse_remembered(&many).len(), MAX_FONTS);
        }
    }
}
