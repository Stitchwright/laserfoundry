//! Downloads the font library: the `Stitchwright/stitchfoundry-fonts` repository. The file list comes
//! from GitHub's tree API and each file from its raw address, so only what was asked for is fetched
//! (the preview pictures are skipped, they are most of the repository's weight).
//!
//! Everything from the network is untrusted: paths are checked before anything is written, sizes and
//! counts are capped, and files are written to a temporary name and renamed into place.
//! GitHub's API: <https://docs.github.com/en/rest/git/trees>

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use thiserror::Error;

/// Most files one download will fetch.
pub const MAX_FILES: usize = 2_000;
/// Largest single file accepted, in bytes.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Most bytes one download will fetch in total.
pub const MAX_TOTAL_BYTES: u64 = 400 * 1024 * 1024;
/// Largest tree listing read, in bytes.
const MAX_TREE_BYTES: u64 = 8 * 1024 * 1024;
/// Deepest folder nesting accepted.
const MAX_DEPTH: usize = 6;
/// Longest path accepted, in bytes.
const MAX_PATH_BYTES: usize = 200;
/// Preview pictures and similar, which the app has no use for.
const SKIPPED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "pdf", "ds_store"];

#[derive(Debug, Error)]
pub enum FontLibError {
    #[error("could not reach the font library: {0}")]
    Network(String),
    #[error("the font library listing is not what was expected: {0}")]
    BadListing(String),
    #[error("the font library is larger than this program will download; refusing")]
    TooLarge,
    #[error("could not write {path}: {reason}")]
    Write { path: String, reason: String },
    #[error("the download was cancelled")]
    Cancelled,
    #[error("no folder is available to keep the fonts in")]
    NoFolder,
}

/// Which part of the library to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Set {
    /// The Foundry fonts under `fonts/` (about a megabyte).
    Foundry,
    /// Those and the Ink/Stitch collection under `inkstitch/` (hundreds of megabytes).
    Everything,
}

impl Set {
    fn prefixes(self) -> &'static [&'static str] {
        match self {
            Set::Foundry => &["fonts/"],
            Set::Everything => &["fonts/", "inkstitch/"],
        }
    }
}

/// Where the listing and the files come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Returns the repository tree as JSON (`?recursive=1` included).
    pub tree_url: String,
    /// File `a/b` is fetched from `{raw_base}/a/b`.
    pub raw_base: String,
}

impl Default for Source {
    fn default() -> Self {
        Source {
            tree_url: "https://api.github.com/repos/Stitchwright/stitchfoundry-fonts/git/trees/main?recursive=1".into(),
            raw_base: "https://raw.githubusercontent.com/Stitchwright/stitchfoundry-fonts/main".into(),
        }
    }
}

/// One file to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Relative, `/`-separated, already checked.
    pub path: String,
    pub size: u64,
}

/// What a download did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub fetched: usize,
    /// Files already present with the right size.
    pub kept: usize,
    pub bytes: u64,
}

/// `LaserFoundry/fonts` in the per-user data folder (`$XDG_DATA_HOME` or `~/.local/share` on Linux,
/// `~/Library/Application Support` on macOS, `%APPDATA%` on Windows).
pub fn default_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
    };
    Some(base?.join("LaserFoundry").join("fonts"))
}

/// The files to fetch from a tree listing: blobs under the set's folders, minus skipped kinds, with
/// unsafe paths refused outright. `Err` when the listing is not a tree or the set is too big.
pub fn plan(tree_json: &str, set: Set) -> Result<Vec<Item>, FontLibError> {
    let v: serde_json::Value = serde_json::from_str(tree_json).map_err(|e| FontLibError::BadListing(e.to_string()))?;
    if v.get("truncated").and_then(|t| t.as_bool()) == Some(true) {
        return Err(FontLibError::BadListing("the listing was cut short".into()));
    }
    let entries = v.get("tree").and_then(|t| t.as_array()).ok_or_else(|| FontLibError::BadListing("no \"tree\" list".into()))?;
    let mut items = Vec::new();
    let mut total = 0u64;
    for entry in entries {
        if entry.get("type").and_then(|t| t.as_str()) != Some("blob") {
            continue;
        }
        let (Some(path), Some(size)) = (entry.get("path").and_then(|p| p.as_str()), entry.get("size").and_then(|s| s.as_u64())) else { continue };
        if !set.prefixes().iter().any(|p| path.starts_with(p)) || is_skipped(path) {
            continue;
        }
        if !safe_path(path) {
            log::warn!("skipping a font library path that is not safe to write: {path:?}");
            continue;
        }
        if size > MAX_FILE_BYTES {
            log::warn!("skipping {path}: {size} bytes is over the {MAX_FILE_BYTES} limit");
            continue;
        }
        total += size;
        if items.len() >= MAX_FILES || total > MAX_TOTAL_BYTES {
            return Err(FontLibError::TooLarge);
        }
        items.push(Item { path: path.to_string(), size });
    }
    Ok(items)
}

/// Fetches the set into `dest` (created as needed). Files already there with the listed size are
/// kept, so a repeat only fetches what is missing. `progress(done, total)` is called as files finish;
/// setting `cancel` stops after the file in hand.
pub fn download(
    dest: &Path,
    source: &Source,
    set: Set,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Summary, FontLibError> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).build().into();
    let tree = read_limited(&agent, &source.tree_url, MAX_TREE_BYTES)?;
    let items = plan(&String::from_utf8_lossy(&tree), set)?;
    let mut summary = Summary::default();
    progress(0, items.len());
    for (done, item) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(FontLibError::Cancelled);
        }
        let target = dest.join(item.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == item.size) {
            summary.kept += 1;
        } else {
            let bytes = read_limited(&agent, &format!("{}/{}", source.raw_base.trim_end_matches('/'), encode_path(&item.path)), item.size.max(1))?;
            if bytes.len() as u64 != item.size {
                return Err(FontLibError::Network(format!("{} arrived as {} bytes, expected {}", item.path, bytes.len(), item.size)));
            }
            write_atomically(&target, &bytes)?;
            summary.fetched += 1;
            summary.bytes += item.size;
        }
        progress(done + 1, items.len());
    }
    Ok(summary)
}

/// Whether a relative path is one we will write: plain names only, no way out of the folder.
pub fn safe_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.starts_with('/') {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() <= MAX_DEPTH
        && parts.iter().all(|p| {
            !p.is_empty()
                && !p.starts_with('.')
                && !p.ends_with([' ', '.'])
                && !p.chars().any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
                && Path::new(p).components().all(|c| matches!(c, Component::Normal(_)))
        })
}

fn is_skipped(path: &str) -> bool {
    let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    path.contains('.') && SKIPPED_EXTENSIONS.contains(&extension.as_str())
}

/// Percent-encodes a path for a URL, keeping `/` and the unreserved characters.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Reads a response body of at most `limit` bytes; a longer one is an error, not a truncation.
fn read_limited(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, FontLibError> {
    let response = agent.get(url).header("User-Agent", "laserfoundry-fontlib").call().map_err(|e| FontLibError::Network(e.to_string()))?;
    let mut body = Vec::new();
    response.into_body().into_reader().take(limit + 1).read_to_end(&mut body).map_err(|e| FontLibError::Network(e.to_string()))?;
    if body.len() as u64 > limit {
        return Err(FontLibError::Network(format!("{url} is larger than the {limit} bytes expected")));
    }
    Ok(body)
}

fn write_atomically(target: &Path, bytes: &[u8]) -> Result<(), FontLibError> {
    let fail = |reason: String| FontLibError::Write { path: target.display().to_string(), reason };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| fail(e.to_string()))?;
    }
    let mut temporary = target.as_os_str().to_owned();
    temporary.push(".part");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, bytes).map_err(|e| fail(e.to_string()))?;
    std::fs::rename(&temporary, target).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        fail(e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    mod plan_tests {
        use super::*;

        fn tree(entries: &[(&str, &str, u64)]) -> String {
            let list: Vec<_> = entries.iter().map(|(p, t, s)| serde_json::json!({ "path": p, "type": t, "size": s })).collect();
            serde_json::json!({ "tree": list, "truncated": false }).to_string()
        }

        #[test]
        fn the_foundry_set_is_only_the_fonts_folder_without_previews() {
            let t = tree(&[
                ("README.md", "blob", 10),
                ("fonts", "tree", 0),
                ("fonts/foundry-block/font.json", "blob", 100),
                ("fonts/foundry-block/preview.png", "blob", 5000),
                ("fonts/foundry-block/OFL.txt", "blob", 40),
                ("inkstitch/abecedaire/ltr.svg", "blob", 900),
            ]);
            let paths: Vec<_> = plan(&t, Set::Foundry).unwrap().into_iter().map(|i| i.path).collect();
            assert_eq!(paths, ["fonts/foundry-block/font.json", "fonts/foundry-block/OFL.txt"]);
            let all = plan(&t, Set::Everything).unwrap();
            assert_eq!(all.len(), 3);
            assert_eq!(all[2].path, "inkstitch/abecedaire/ltr.svg");
        }

        #[test]
        fn unsafe_oversized_and_excessive_entries_are_handled() {
            let t = tree(&[
                ("fonts/../../etc/passwd", "blob", 1),
                ("fonts//x", "blob", 1),
                ("fonts/.hidden/a", "blob", 1),
                ("fonts/a\\b", "blob", 1),
                ("fonts/C:evil", "blob", 1),
                ("fonts/huge.json", "blob", MAX_FILE_BYTES + 1),
                ("fonts/ok/font.json", "blob", 1),
                ("fonts/inclusif(ve)/mode d'emploi.txt", "blob", 1),
            ]);
            let paths: Vec<_> = plan(&t, Set::Foundry).unwrap().into_iter().map(|i| i.path).collect();
            assert_eq!(paths, ["fonts/ok/font.json", "fonts/inclusif(ve)/mode d'emploi.txt"]);

            let many: Vec<(String, &str, u64)> = (0..MAX_FILES + 1).map(|i| (format!("fonts/f{i}.json"), "blob", 1)).collect();
            let many: Vec<(&str, &str, u64)> = many.iter().map(|(p, t, s)| (p.as_str(), *t, *s)).collect();
            assert!(matches!(plan(&tree(&many), Set::Foundry), Err(FontLibError::TooLarge)));
            let big: Vec<(String, &str, u64)> = (0..30).map(|i| (format!("fonts/f{i}.svg"), "blob", MAX_FILE_BYTES)).collect();
            let big: Vec<(&str, &str, u64)> = big.iter().map(|(p, t, s)| (p.as_str(), *t, *s)).collect();
            assert!(matches!(plan(&tree(&big), Set::Foundry), Err(FontLibError::TooLarge)));
        }

        #[test]
        fn a_listing_that_is_not_a_tree_is_an_error() {
            for text in ["", "not json", "[]", "{}", r#"{"tree": 3}"#, r#"{"tree": [], "truncated": true}"#] {
                assert!(plan(text, Set::Foundry).is_err(), "{text:?}");
            }
            assert_eq!(
                plan(r#"{"tree":[{"type":"blob"},{"path":5,"type":"blob","size":1},{"path":"fonts/a","type":"blob","size":-1}]}"#, Set::Foundry)
                    .unwrap(),
                []
            );
        }
    }

    mod path_tests {
        use super::*;

        #[test]
        fn safe_paths_are_plain_names_inside_the_folder() {
            for ok in ["fonts/a/font.json", "inkstitch/inclusif(ve)/LICENSE", "fonts/mode d'emploi.txt"] {
                assert!(safe_path(ok), "{ok}");
            }
            for bad in [
                "",
                "/abs",
                "a/../b",
                "a/./b",
                "a//b",
                ".git/config",
                "a/b/",
                "a/CON:",
                "a/b\\c",
                "a/x\0y",
                "a/tab\t",
                &"a/".repeat(10),
                &"x".repeat(300),
                "a/b.",
                "a/b ",
            ] {
                assert!(!safe_path(bad), "{bad:?}");
            }
        }

        #[test]
        fn urls_are_percent_encoded() {
            assert_eq!(encode_path("inkstitch/inclusif(ve)/mode d'emploi.txt"), "inkstitch/inclusif%28ve%29/mode%20d%27emploi.txt");
            assert_eq!(encode_path("a/é"), "a/%C3%A9");
        }

        #[test]
        fn the_default_folder_is_under_the_data_folder() {
            if let Some(dir) = default_dir() {
                assert!(dir.ends_with("LaserFoundry/fonts") || dir.ends_with("LaserFoundry\\fonts"), "{dir:?}");
            }
        }
    }

    mod download_tests {
        use super::*;
        use std::collections::HashMap;
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        /// A tiny HTTP server: path -> body (a missing path is a 404). Returns its address and the paths requested.
        fn serve(files: HashMap<String, Vec<u8>>) -> (String, Arc<Mutex<Vec<String>>>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = Arc::clone(&seen);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        continue;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) <= 2 {
                            break;
                        }
                    }
                    log.lock().unwrap().push(path.clone());
                    let mut stream = reader.into_inner();
                    let key = path.split('?').next().unwrap_or("").to_string();
                    let _ = match files.get(&key) {
                        Some(body) => {
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            stream.write_all(body)
                        }
                        None => write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    };
                }
            });
            (address, seen)
        }

        fn library(extra: &[(&str, &[u8])]) -> (Source, Arc<Mutex<Vec<String>>>) {
            let mut files: HashMap<String, Vec<u8>> = HashMap::new();
            let mut entries = Vec::new();
            for (path, body) in
                [("fonts/foundry-block/font.json", &b"{\"name\":\"Block\"}"[..]), ("fonts/foundry-block/glyphs.stitchfoundry", &b"glyphs"[..])]
                    .iter()
                    .chain(extra)
            {
                files.insert(format!("/raw/{}", encode_path(path)), body.to_vec());
                entries.push(serde_json::json!({ "path": path, "type": "blob", "size": body.len() }));
            }
            files.insert("/tree".into(), serde_json::json!({ "tree": entries, "truncated": false }).to_string().into_bytes());
            let (address, seen) = serve(files);
            (Source { tree_url: format!("{address}/tree?recursive=1"), raw_base: format!("{address}/raw") }, seen)
        }

        fn temp_dir(tag: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("laserfoundry-fontlib-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            dir
        }

        #[test]
        fn files_are_fetched_into_place_and_a_repeat_only_fetches_what_is_missing() {
            let (source, seen) = library(&[("fonts/inclusif(ve)/mode d'emploi.txt", b"hello")]);
            let dest = temp_dir("fetch");
            let mut steps = Vec::new();
            let first = download(&dest, &source, Set::Foundry, &AtomicBool::new(false), &mut |d, t| steps.push((d, t))).unwrap();
            assert_eq!((first.fetched, first.kept), (3, 0));
            assert_eq!(steps.first(), Some(&(0, 3)));
            assert_eq!(steps.last(), Some(&(3, 3)));
            assert_eq!(std::fs::read(dest.join("fonts/foundry-block/font.json")).unwrap(), b"{\"name\":\"Block\"}");
            assert_eq!(std::fs::read(dest.join("fonts/inclusif(ve)/mode d'emploi.txt")).unwrap(), b"hello");
            assert!(!dest.join("fonts/foundry-block/font.json.part").exists());

            seen.lock().unwrap().clear();
            std::fs::remove_file(dest.join("fonts/foundry-block/glyphs.stitchfoundry")).unwrap();
            let second = download(&dest, &source, Set::Foundry, &AtomicBool::new(false), &mut |_, _| {}).unwrap();
            assert_eq!((second.fetched, second.kept), (1, 2));
            let requests = seen.lock().unwrap().clone();
            assert_eq!(requests.len(), 2, "the listing and the one missing file: {requests:?}");
            let _ = std::fs::remove_dir_all(&dest);
        }

        #[test]
        fn failures_are_errors_and_leave_no_half_written_files() {
            // A file the server does not have.
            let (mut source, _) = library(&[]);
            source.raw_base.push_str("/missing");
            let dest = temp_dir("missing");
            assert!(matches!(download(&dest, &source, Set::Foundry, &AtomicBool::new(false), &mut |_, _| {}), Err(FontLibError::Network(_))));
            assert!(!dest.join("fonts/foundry-block/font.json").exists());

            // A listing the server does not have, and one nobody is listening for.
            let (mut source, _) = library(&[]);
            source.tree_url = source.tree_url.replace("/tree", "/gone");
            assert!(matches!(download(&dest, &source, Set::Foundry, &AtomicBool::new(false), &mut |_, _| {}), Err(FontLibError::Network(_))));
            let nobody = Source { tree_url: "http://127.0.0.1:9/tree".into(), raw_base: "http://127.0.0.1:9".into() };
            assert!(download(&dest, &nobody, Set::Foundry, &AtomicBool::new(false), &mut |_, _| {}).is_err());
            let _ = std::fs::remove_dir_all(&dest);
        }

        #[test]
        fn a_file_of_the_wrong_size_is_refused_and_cancelling_stops_the_download() {
            let mut files = HashMap::new();
            files.insert("/tree".to_string(), br#"{"tree":[{"path":"fonts/a/font.json","type":"blob","size":3}]}"#.to_vec());
            files.insert("/raw/fonts/a/font.json".to_string(), b"far too long".to_vec());
            let (address, _) = serve(files);
            let source = Source { tree_url: format!("{address}/tree"), raw_base: format!("{address}/raw") };
            let dest = temp_dir("size");
            assert!(matches!(download(&dest, &source, Set::Foundry, &AtomicBool::new(false), &mut |_, _| {}), Err(FontLibError::Network(_))));
            assert!(!dest.join("fonts/a/font.json").exists());

            let (source, _) = library(&[]);
            assert!(matches!(download(&dest, &source, Set::Foundry, &AtomicBool::new(true), &mut |_, _| {}), Err(FontLibError::Cancelled)));
            let _ = std::fs::remove_dir_all(&dest);
        }
    }
}
