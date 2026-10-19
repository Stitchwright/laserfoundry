//! The native `.laserfoundry` file: versioned JSON, validated on load, saved atomically.

use std::fs;
use std::io::Write;
use std::path::Path;

use laserfoundry_doc::{Document, ShapeKind};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const FORMAT_NAME: &str = "laserfoundry";
pub const FORMAT_VERSION: u32 = 1;
pub const EXTENSION: &str = "laserfoundry";
pub const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("could not read or write the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("file is larger than {} MB", MAX_FILE_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("not a LaserFoundry file: {0}")]
    Parse(String),
    #[error("this file was saved by a newer LaserFoundry (format v{0}); update to open it")]
    NewerVersion(u32),
    #[error("file is not a LaserFoundry document")]
    WrongFormat,
    #[error("invalid document: {0}")]
    Invalid(&'static str),
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    document: Document,
}

pub fn to_bytes(doc: &Document) -> Result<Vec<u8>, FormatError> {
    let env = Envelope { format: FORMAT_NAME.into(), version: FORMAT_VERSION, document: doc.clone() };
    serde_json::to_vec(&env).map_err(|e| FormatError::Parse(e.to_string()))
}

pub fn from_bytes(bytes: &[u8]) -> Result<Document, FormatError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(FormatError::TooLarge);
    }
    // Read the header first so a newer file gives a helpful error instead of a schema mismatch.
    let header: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| FormatError::Parse(e.to_string()))?;
    if header.get("format").and_then(|v| v.as_str()) != Some(FORMAT_NAME) {
        return Err(FormatError::WrongFormat);
    }
    let version = header.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version > FORMAT_VERSION {
        return Err(FormatError::NewerVersion(version));
    }
    let env: Envelope = serde_json::from_value(header).map_err(|e| FormatError::Parse(e.to_string()))?;
    let mut doc = env.document;
    validate(&mut doc)?;
    Ok(doc)
}

/// Writes to a sibling temp file, then renames, so a crash never leaves a half-written project.
pub fn save(path: &Path, doc: &Document) -> Result<(), FormatError> {
    let bytes = to_bytes(doc)?;
    // A name nobody can guess, created fresh (never through a link someone left behind).
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tmp = path.with_extension(format!("{EXTENSION}.{}.{nanos}.tmp", std::process::id()));
    let written = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written.and_then(|()| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

pub fn load(path: &Path) -> Result<Document, FormatError> {
    match read_capped(path, MAX_FILE_BYTES as u64)? {
        Some(bytes) => from_bytes(&bytes),
        None => Err(FormatError::TooLarge),
    }
}

/// Reads at most `limit` bytes of a file; `None` when there is more. The size is not taken from the file's
/// metadata, which says 0 for devices and pipes (`/dev/zero` would otherwise be read until memory ran out).
pub fn read_capped(path: &Path, limit: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// Repairs what can be repaired (dangling layers, id counter) and rejects what cannot.
fn validate(doc: &mut Document) -> Result<(), FormatError> {
    let m = &doc.machine;
    if !(m.bed_width.is_finite() && m.bed_width > 0.0 && m.bed_height.is_finite() && m.bed_height > 0.0) {
        return Err(FormatError::Invalid("bed size must be positive"));
    }
    if m.max_s == 0 {
        return Err(FormatError::Invalid("max S value must be positive"));
    }
    if doc.layers.is_empty() {
        return Err(FormatError::Invalid("document has no layers"));
    }
    let first_layer = doc.layers.first().map(|l| l.id).unwrap_or(0);
    let layer_ids: Vec<_> = doc.layers.iter().map(|l| l.id).collect();
    for s in &mut doc.shapes {
        if !layer_ids.contains(&s.layer) {
            s.layer = first_layer;
        }
        if let ShapeKind::Image { image, .. } = &s.kind
            && (image.width as usize).checked_mul(image.height as usize) != Some(image.gray.len())
        {
            return Err(FormatError::Invalid("image size does not match its pixel data"));
        }
        if let ShapeKind::Image { image, .. } = &s.kind
            && image.rgb.as_ref().is_some_and(|rgb| rgb.len() != image.gray.len().saturating_mul(3))
        {
            return Err(FormatError::Invalid("image colour data does not match its size"));
        }
    }
    let max_id = doc.shapes.iter().map(|s| s.id).max().unwrap_or(0);
    doc.next_id = doc.next_id.max(max_id.saturating_add(1));
    Ok(())
}

#[cfg(test)]
mod tests {
    use laserfoundry_doc::Shape;
    use laserfoundry_geom::rect_path;

    use super::*;

    #[cfg(unix)]
    #[test]
    fn reading_is_capped_even_for_files_that_claim_to_be_empty() {
        // /dev/zero reports a length of 0 and never ends: it used to be read until memory ran out.
        assert_eq!(read_capped(Path::new("/dev/zero"), 1000).unwrap(), None);
        assert!(matches!(load(Path::new("/dev/zero")), Err(FormatError::TooLarge)));
        let dir = std::env::temp_dir().join(format!("lf-format-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("five.bin");
        std::fs::write(&file, b"12345").unwrap();
        assert_eq!(read_capped(&file, 5).unwrap().as_deref(), Some(&b"12345"[..]));
        assert_eq!(read_capped(&file, 4).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn saving_never_writes_through_a_planted_link_at_the_old_temp_name() {
        let dir = std::env::temp_dir().join(format!("lf-format-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, "precious").unwrap();
        let path = dir.join("x.laserfoundry");
        std::os::unix::fs::symlink(&victim, dir.join("x.laserfoundry.tmp")).unwrap();
        save(&path, &sample()).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
        assert_eq!(load(&path).unwrap().shapes.len(), 1);
        let leftovers: Vec<_> =
            std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(leftovers.iter().all(|n| !n.ends_with(".tmp") || n == "x.laserfoundry.tmp"), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sample() -> Document {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("r", 3, rect_path(1.0, 2.0, 3.0, 4.0)));
        doc
    }

    #[test]
    fn a_picture_whose_colour_data_does_not_fit_is_refused() {
        use laserfoundry_doc::{ImageAdjust, RasterImage, Shape, ShapeKind};
        let mut doc = sample();
        let mut shape = Shape::path("p", 0, laserfoundry_geom::rect_path(0.0, 0.0, 1.0, 1.0));
        let image = RasterImage::new(2, 1, vec![1, 2]).unwrap();
        shape.kind = ShapeKind::Image {
            image: std::sync::Arc::new(image.clone().with_rgb(vec![0; 6]).unwrap()),
            width_mm: 2.0,
            height_mm: 1.0,
            adjust: ImageAdjust::default(),
        };
        doc.add_shape(shape.clone());
        assert!(from_bytes(&to_bytes(&doc).unwrap()).is_ok());
        let mut bad = image;
        bad.rgb = Some(vec![0; 5]);
        shape.kind = ShapeKind::Image { image: std::sync::Arc::new(bad), width_mm: 2.0, height_mm: 1.0, adjust: ImageAdjust::default() };
        doc.add_shape(shape);
        assert!(from_bytes(&to_bytes(&doc).unwrap()).is_err());
    }

    #[test]
    fn bytes_round_trip() {
        let doc = sample();
        assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    }

    #[test]
    fn save_and_load_are_atomic_and_leave_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("lf-format-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.laserfoundry");
        save(&path, &sample()).unwrap();
        assert_eq!(load(&path).unwrap(), sample());
        assert!(!dir.join("a.laserfoundry.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_and_foreign_files_are_rejected_politely() {
        assert!(matches!(from_bytes(br#"{"format":"laserfoundry","version":99,"document":{}}"#), Err(FormatError::NewerVersion(99))));
        assert!(matches!(from_bytes(br#"{"format":"other","version":1}"#), Err(FormatError::WrongFormat)));
        assert!(from_bytes(b"not json").is_err());
        assert!(from_bytes(b"").is_err());
    }

    #[test]
    fn dangling_layers_are_repaired_and_bad_machines_rejected() {
        let mut doc = sample();
        doc.shapes[0].layer = 77;
        let back = from_bytes(&to_bytes(&doc).unwrap()).unwrap();
        assert_eq!(back.shapes[0].layer, 0);

        let mut bad = sample();
        bad.machine.bed_width = -1.0;
        assert!(matches!(from_bytes(&to_bytes(&bad).unwrap()), Err(FormatError::Invalid(_))));
    }

    #[test]
    fn id_counter_never_collides_with_existing_shapes() {
        let mut doc = sample();
        doc.next_id = 1;
        let back = from_bytes(&to_bytes(&doc).unwrap()).unwrap();
        assert!(back.next_id > back.shapes[0].id);
    }
}
