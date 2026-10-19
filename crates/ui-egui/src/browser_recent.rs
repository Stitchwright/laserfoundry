//! Recently opened or saved projects in a browser, which has no file paths to remember: the
//! project's bytes are kept instead, in the page's small text storage, within a fixed budget.

use laserfoundry_doc::base64;

/// Projects kept.
pub const MAX_ENTRIES: usize = 8;
/// A project whose stored (base64) text is longer than this is not remembered.
pub const MAX_ENTRY_CHARS: usize = 1_000_000;
/// Everything stored together stays under this, so it fits the browser's storage quota.
pub const MAX_TOTAL_CHARS: usize = 3_000_000;
/// Longest name kept, in characters.
const MAX_NAME_CHARS: usize = 120;
/// Stored text longer than this is not read (it is not ours).
const MAX_LOAD_CHARS: usize = MAX_TOTAL_CHARS + 100_000;

/// The key the list lives under in the platform's storage.
pub const STORAGE_KEY: &str = "laserfoundry.recent";

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    /// The project file, base64 encoded.
    pub data: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserRecent {
    /// Most recent first.
    pub entries: Vec<Entry>,
}

impl BrowserRecent {
    /// Reads the stored list; missing, oversized or damaged text gives an empty one, and entries
    /// that do not fit the budget are dropped.
    pub fn parse(text: &str) -> BrowserRecent {
        if text.len() > MAX_LOAD_CHARS {
            return BrowserRecent::default();
        }
        let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(text) else { return BrowserRecent::default() };
        let mut recent = BrowserRecent::default();
        for item in list.iter().take(MAX_ENTRIES) {
            let (Some(name), Some(data)) = (item.get("name").and_then(|n| n.as_str()), item.get("data").and_then(|d| d.as_str())) else { continue };
            if !name.is_empty() && data.len() <= MAX_ENTRY_CHARS && recent.total_chars() + data.len() <= MAX_TOTAL_CHARS {
                recent.entries.push(Entry { name: name.chars().take(MAX_NAME_CHARS).collect(), data: data.to_string() });
            }
        }
        recent
    }

    pub fn to_text(&self) -> String {
        let list: Vec<serde_json::Value> = self.entries.iter().map(|e| serde_json::json!({ "name": e.name, "data": e.data })).collect();
        serde_json::to_string(&list).unwrap_or_else(|_| "[]".into())
    }

    /// Moves the project called `name` to the front. Returns false (and changes nothing) when it is
    /// too large to remember. Older entries are dropped as needed to stay within the budget.
    pub fn push(&mut self, name: &str, bytes: &[u8]) -> bool {
        let data = base64::encode(bytes);
        if name.is_empty() || data.len() > MAX_ENTRY_CHARS {
            return false;
        }
        let name: String = name.chars().take(MAX_NAME_CHARS).collect();
        self.entries.retain(|e| e.name != name);
        self.entries.insert(0, Entry { name, data });
        self.entries.truncate(MAX_ENTRIES);
        while self.total_chars() > MAX_TOTAL_CHARS && self.entries.len() > 1 {
            self.entries.pop();
        }
        true
    }

    /// The stored project called `name`, decoded.
    pub fn bytes(&self, name: &str) -> Option<Vec<u8>> {
        self.entries.iter().find(|e| e.name == name).and_then(|e| base64::decode(&e.data))
    }

    pub fn remove(&mut self, name: &str) {
        self.entries.retain(|e| e.name != name);
    }

    fn total_chars(&self) -> usize {
        self.entries.iter().map(|e| e.data.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod push_tests {
        use super::*;

        #[test]
        fn the_newest_goes_first_and_a_repeat_replaces_the_older_copy() {
            let mut r = BrowserRecent::default();
            assert!(r.push("a.laserfoundry", b"one"));
            assert!(r.push("b.laserfoundry", b"two"));
            assert!(r.push("a.laserfoundry", b"three"));
            let names: Vec<_> = r.entries.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, ["a.laserfoundry", "b.laserfoundry"]);
            assert_eq!(r.bytes("a.laserfoundry").as_deref(), Some(&b"three"[..]));
            assert_eq!(r.bytes("missing"), None);
        }

        #[test]
        fn only_the_newest_entries_and_the_budget_are_kept() {
            let mut r = BrowserRecent::default();
            for i in 0..20 {
                assert!(r.push(&format!("p{i}.laserfoundry"), b"x"));
            }
            assert_eq!(r.entries.len(), MAX_ENTRIES);
            assert_eq!(r.entries[0].name, "p19.laserfoundry");

            // Each just under the entry limit: only as many as fit the total survive.
            let big = vec![7u8; MAX_ENTRY_CHARS / 4 * 3 - 10];
            let mut r = BrowserRecent::default();
            for i in 0..6 {
                assert!(r.push(&format!("big{i}"), &big));
            }
            assert!(r.total_chars() <= MAX_TOTAL_CHARS, "{}", r.total_chars());
            assert_eq!(r.entries[0].name, "big5", "the newest always stays");
        }

        #[test]
        fn a_project_that_is_too_large_or_unnamed_is_refused_and_changes_nothing() {
            let mut r = BrowserRecent::default();
            r.push("keep", b"x");
            assert!(!r.push("huge", &vec![0u8; MAX_ENTRY_CHARS]));
            assert!(!r.push("", b"x"));
            assert_eq!(r.entries.len(), 1);
            r.remove("keep");
            assert!(r.entries.is_empty());
        }
    }

    mod storage_tests {
        use super::*;

        #[test]
        fn the_list_survives_a_round_trip() {
            let mut r = BrowserRecent::default();
            r.push("a.laserfoundry", &[0, 1, 2, 255]);
            r.push("b.laserfoundry", b"hello");
            let back = BrowserRecent::parse(&r.to_text());
            assert_eq!(back, r);
            assert_eq!(back.bytes("a.laserfoundry"), Some(vec![0, 1, 2, 255]));
        }

        #[test]
        fn damaged_or_hostile_text_gives_an_empty_or_trimmed_list() {
            for text in ["", "not json", "{}", "[1,2,3]", r#"[{"name":1,"data":2}]"#, r#"[{"name":"","data":"AA=="}]"#, "null"] {
                assert!(BrowserRecent::parse(text).entries.is_empty(), "{text:?}");
            }
            assert!(BrowserRecent::parse(&"[".repeat(MAX_LOAD_CHARS + 1)).entries.is_empty());
            let many = format!("[{}]", vec![r#"{"name":"n","data":"AA=="}"#; 100].join(","));
            assert_eq!(BrowserRecent::parse(&many).entries.len(), MAX_ENTRIES);
            let long_name = format!(r#"[{{"name":"{}","data":"AA=="}}]"#, "n".repeat(10_000));
            assert_eq!(BrowserRecent::parse(&long_name).entries[0].name.chars().count(), MAX_NAME_CHARS);
            let oversize = format!(r#"[{{"name":"big","data":"{}"}}]"#, "A".repeat(MAX_ENTRY_CHARS + 1));
            assert!(BrowserRecent::parse(&oversize).entries.is_empty());
        }

        #[test]
        fn undecodable_stored_data_gives_no_bytes() {
            let r = BrowserRecent::parse(r#"[{"name":"bad","data":"!!! not base64 !!!"}]"#);
            assert_eq!(r.bytes("bad"), None);
        }
    }
}
