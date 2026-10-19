//! Keeping a few older copies of the autosave, and checking that what was written can be read.
//!
//! The newest autosave lives at the platform's recovery path; the copies before it sit beside it as
//! `name.1.ext`, `name.2.ext`, and so on (1 is the most recent older copy).

use std::path::{Path, PathBuf};

/// Autosave copies kept, newest included.
pub const GENERATIONS: usize = 3;

/// Path of generation `n` (0 is the recovery file itself).
pub fn generation_path(base: &Path, n: usize) -> PathBuf {
    if n == 0 {
        return base.to_path_buf();
    }
    let stem = base.file_stem().map_or_else(|| "recovery".to_string(), |s| s.to_string_lossy().into_owned());
    let name = match base.extension() {
        Some(ext) => format!("{stem}.{n}.{}", ext.to_string_lossy()),
        None => format!("{stem}.{n}"),
    };
    base.with_file_name(name)
}

/// Shifts every copy one place older (dropping the oldest) so `base` is free for a new autosave.
/// Failures are logged; losing an old copy must never stop the new one being written.
pub fn rotate(base: &Path) {
    for n in (0..GENERATIONS).rev() {
        let from = generation_path(base, n);
        if !from.exists() {
            continue;
        }
        let to = generation_path(base, n + 1);
        let result = if n + 1 >= GENERATIONS { std::fs::remove_file(&from) } else { std::fs::rename(&from, &to) };
        if let Err(e) = result {
            log::warn!("could not rotate {}: {e}", from.display());
        }
    }
}

/// Removes the recovery file and every older copy.
pub fn remove_all(base: &Path) {
    for n in 0..GENERATIONS {
        let p = generation_path(base, n);
        if let Err(e) = std::fs::remove_file(&p)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not remove {}: {e}", p.display());
        }
    }
}

/// Whether the file reads back as a project.
pub fn is_readable(path: &Path) -> bool {
    laserfoundry_format::load(path).is_ok()
}

/// The newest copy that reads back as a project, if any exist.
pub fn newest_readable(base: &Path) -> Option<PathBuf> {
    (0..GENERATIONS).map(|n| generation_path(base, n)).find(|p| p.exists() && is_readable(p))
}
