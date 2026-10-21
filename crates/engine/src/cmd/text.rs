use laserfoundry_doc::{LayerId, Shape, ShapeId, ShapeKind, TextAlign, TextSettings};
use laserfoundry_geom::Affine;
use laserfoundry_text::{Align, TextStyle, layout};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, parse, shapes::finite};
use crate::{EngineError, Session};

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "text.create",
        label: "Text",
        menu: "Create",
        shortcut: Some("T"),
        params_doc: "{text, x, y, height? (cap height mm, default 10), letter_spacing?, line_spacing?, align?: left|center|right, layer?}  (single-line stroke font), font? (path to a .ttf/.otf file, a font library folder (Foundry or Ink/Stitch), or library:<folder> for a font a browser host loaded: draws filled outlines; use a Fill layer to engrave them), vertical? (stack letters top to bottom, one column per line, first line rightmost), fallback_font? (path to a second .ttf/.otf for characters the font lacks)",
        mutates: true,
        run: create,
    },
    CommandSpec {
        id: "text.on_path",
        label: "Text on Path",
        menu: "Create",
        shortcut: None,
        params_doc: "{text: id of a text shape, path: id of the shape to follow, offset? (mm along the path, default 0), keep? (keep the originals, default true)}  — makes a new path shape with the letters bent along the line; it is plain geometry afterwards, not editable text",
        mutates: true,
        run: on_path,
    },
    CommandSpec {
        id: "text.preview",
        label: "Preview Font",
        menu: "",
        shortcut: None,
        params_doc: "{font? (as for text.create; omit for the built-in font), text? (a sample, at most 100 characters, default \"Aa Bb Cc 123\"), height? (cap height mm, 1 to 100, default 10)}  \
                     — changes nothing; returns {paths: [{closed, points: [[x, y]]}], width, height, unsupported}, the sample's outlines in mm from its top-left corner",
        mutates: false,
        run: preview,
    },
    CommandSpec {
        id: "text.edit",
        label: "Edit Text",
        menu: "Edit",
        shortcut: None,
        params_doc: "{id? (default: the selected text), text?, height?, letter_spacing?, line_spacing?, align?, font? (path, or \"\" for the built-in font), vertical?, fallback_font? (path, or \"\" for none)}",
        mutates: true,
        run: edit,
    },
];

#[derive(Deserialize)]
struct CreateParams {
    text: String,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    height: Option<f64>,
    letter_spacing: Option<f64>,
    line_spacing: Option<f64>,
    align: Option<TextAlign>,
    font: Option<String>,
    vertical: Option<bool>,
    fallback_font: Option<String>,
    #[serde(default)]
    layer: LayerId,
}

fn create(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: CreateParams = parse(params)?;
    finite(&[p.x, p.y])?;
    let defaults = TextSettings::default();
    let settings = TextSettings {
        height: p.height.unwrap_or(defaults.height),
        letter_spacing: p.letter_spacing.unwrap_or(defaults.letter_spacing),
        line_spacing: p.line_spacing.unwrap_or(defaults.line_spacing),
        align: p.align.unwrap_or(defaults.align),
        font: p.font.filter(|f| !f.is_empty()),
        vertical: p.vertical.unwrap_or(defaults.vertical),
        fallback_font: p.fallback_font.filter(|f| !f.is_empty()),
    };
    let (path, unsupported) = generate(&p.text, &settings)?;
    let name = p.text.lines().next().unwrap_or("Text").chars().take(24).collect::<String>();
    let shape = Shape {
        id: 0,
        name,
        layer: p.layer,
        transform: Affine::translate((p.x, p.y)),
        kind: ShapeKind::Text { content: p.text, style: settings, path },
        cut: None,
        group: None,
        order: 0,
    };
    let id = s.doc.add_shape(shape);
    s.selection = vec![id];
    Ok(json!({ "id": id, "unsupported": unsupported }))
}

/// Longest sample `text.preview` lays out, in characters.
const MAX_PREVIEW_CHARS: usize = 100;
const PREVIEW_SAMPLE: &str = "Aa Bb Cc 123";

#[derive(Deserialize)]
struct PreviewParams {
    font: Option<String>,
    text: Option<String>,
    height: Option<f64>,
}

fn preview(_: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: PreviewParams = parse(params)?;
    let text = p.text.unwrap_or_else(|| PREVIEW_SAMPLE.to_string());
    if text.chars().count() > MAX_PREVIEW_CHARS {
        return Err(EngineError::BadParams(format!("a preview sample is at most {MAX_PREVIEW_CHARS} characters")));
    }
    let height = p.height.unwrap_or(10.0);
    if !(height.is_finite() && (1.0..=100.0).contains(&height)) {
        return Err(EngineError::BadParams("the preview height is 1 to 100 mm".into()));
    }
    let settings = TextSettings { height, font: p.font.filter(|f| !f.is_empty()), ..TextSettings::default() };
    let (path, unsupported) = generate(&text, &settings)?;
    let lines = laserfoundry_geom::flatten(&path, 0.05);
    let bounds = laserfoundry_geom::bounds_of_polylines(&lines);
    let (x0, y0) = bounds.map_or((0.0, 0.0), |b| (b.x0, b.y0));
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    let paths: Vec<Value> = lines
        .iter()
        .map(|l| json!({ "closed": l.closed, "points": l.points.iter().map(|pt| [round(pt.x - x0), round(pt.y - y0)]).collect::<Vec<_>>() }))
        .collect();
    Ok(
        json!({ "paths": paths, "width": bounds.map_or(0.0, |b| round(b.width())), "height": bounds.map_or(0.0, |b| round(b.height())), "unsupported": unsupported }),
    )
}

#[derive(Deserialize)]
struct EditParams {
    id: Option<ShapeId>,
    text: Option<String>,
    height: Option<f64>,
    letter_spacing: Option<f64>,
    line_spacing: Option<f64>,
    align: Option<TextAlign>,
    font: Option<String>,
    vertical: Option<bool>,
    fallback_font: Option<String>,
}

fn edit(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: EditParams = parse(params)?;
    let id = match p.id {
        Some(id) => id,
        None => match s.selection.as_slice() {
            [only] => *only,
            _ => return Err(EngineError::BadParams("select exactly one text object, or pass an id".into())),
        },
    };
    let shape = s.doc.shape_mut(id).ok_or_else(|| EngineError::NotFound(format!("shape {id} does not exist")))?;
    let ShapeKind::Text { content, style, path } = &mut shape.kind else {
        return Err(EngineError::BadParams(format!("shape {id} is not text")));
    };
    let mut new_style = style.clone();
    new_style.height = p.height.unwrap_or(new_style.height);
    new_style.letter_spacing = p.letter_spacing.unwrap_or(new_style.letter_spacing);
    new_style.line_spacing = p.line_spacing.unwrap_or(new_style.line_spacing);
    new_style.align = p.align.unwrap_or(new_style.align);
    if let Some(f) = p.font {
        new_style.font = (!f.is_empty()).then_some(f);
    }
    new_style.vertical = p.vertical.unwrap_or(new_style.vertical);
    if let Some(f) = p.fallback_font {
        new_style.fallback_font = (!f.is_empty()).then_some(f);
    }
    let new_content = p.text.unwrap_or_else(|| content.clone());
    let (new_path, unsupported) = generate(&new_content, &new_style)?;
    *content = new_content;
    *style = new_style;
    *path = new_path;
    Ok(json!({ "id": id, "unsupported": unsupported }))
}

pub(super) fn generate(text: &str, s: &TextSettings) -> Result<(laserfoundry_geom::BezPath, Vec<char>), EngineError> {
    if text.trim().is_empty() {
        return Err(EngineError::BadParams("text is empty".into()));
    }
    let style = TextStyle {
        height: s.height,
        letter_spacing: s.letter_spacing,
        line_spacing: s.line_spacing,
        align: match s.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        },
    };
    let font = s.font.as_deref().map(read_font).transpose()?;
    // A fallback only applies to a file font; a font folder from the font library has none.
    let fallback = match (&font, s.fallback_font.as_deref()) {
        (Some(LoadedFont::File(_)), Some(path)) => match read_font(path)? {
            LoadedFont::File(data) => Some(data),
            LoadedFont::Glyphs(_) => return Err(EngineError::BadParams("a font library folder cannot be used as a fallback font".into())),
        },
        _ => None,
    };
    let one = |t: &str, st: &TextStyle| match &font {
        Some(LoadedFont::File(data)) => laserfoundry_text::layout_outline_with_fallback(data, fallback.as_deref(), t, st),
        Some(LoadedFont::Glyphs(font)) => laserfoundry_text::layout_glyph_font(font, t, st),
        None => layout(t, st),
    };
    let l = if s.vertical { laserfoundry_text::layout_vertical(text, &style, |c| one(c, &style)) } else { one(text, &style) }
        .map_err(|e| EngineError::BadParams(e.to_string()))?;
    Ok((l.path, l.unsupported))
}

/// A font as loaded: a TrueType/OpenType file's bytes, or a font library folder's ready-drawn glyphs.
enum LoadedFont {
    File(Vec<u8>),
    Glyphs(std::sync::Arc<laserfoundry_text::GlyphFont>),
}

/// The font library's folders: `font.json` with either `glyphs.stitchfoundry` (the Foundry fonts) or
/// `ltr.svg` (the Ink/Stitch collection), named by the folder or by one of its files.
const GLYPH_FONT_FILES: [&str; 3] = ["font.json", "glyphs.stitchfoundry", "ltr.svg"];

/// Reads a font, refusing anything unreasonably large before it is loaded.
fn read_font(path: &str) -> Result<LoadedFont, EngineError> {
    // A name the host loaded into memory (the browser has no folders to point at).
    if path.starts_with(crate::fontlib::PREFIX) {
        return crate::fontlib::get(path)
            .map(LoadedFont::Glyphs)
            .ok_or_else(|| EngineError::BadParams(format!("the library font {path} is not loaded; download the font library first")));
    }
    let p = std::path::Path::new(path);
    let folder =
        if p.is_dir() { Some(p) } else { p.file_name().and_then(|n| n.to_str()).filter(|n| GLYPH_FONT_FILES.contains(n)).and_then(|_| p.parent()) };
    match folder {
        Some(folder) if folder.join("font.json").is_file() && (folder.join("glyphs.stitchfoundry").is_file() || folder.join("ltr.svg").is_file()) => {
            read_glyph_font(folder)
        }
        // A font library folder this program cannot read yet: say so, rather than failing to open a folder as a file.
        Some(folder) if folder.join("font.json").is_file() => Err(EngineError::BadParams(format!(
            "{} has no ltr.svg or glyphs.stitchfoundry (fonts that are right-to-left only, or split into one file per letter, are not supported yet)",
            folder.display()
        ))),
        _ => read_font_file(path).map(LoadedFont::File),
    }
}

/// Parsed library fonts, so laying text out again (every edit) does not parse a drawing of many
/// megabytes each time. Keyed by folder; an entry is stale when either file's size or time changed.
type FontCache = std::sync::Mutex<Vec<(std::path::PathBuf, [(u64, Option<std::time::SystemTime>); 2], std::sync::Arc<laserfoundry_text::GlyphFont>)>>;
const CACHED_FONTS: usize = 8;

fn font_cache() -> &'static FontCache {
    static CACHE: std::sync::OnceLock<FontCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

fn read_glyph_font(folder: &std::path::Path) -> Result<LoadedFont, EngineError> {
    let inkstitch = !folder.join("glyphs.stitchfoundry").is_file();
    let second = if inkstitch { "ltr.svg" } else { "glyphs.stitchfoundry" };
    let signature = |name: &str| std::fs::metadata(folder.join(name)).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
    let sig = [signature("font.json"), signature(second)];
    if let Ok(cache) = font_cache().lock()
        && let Some((_, _, font)) = cache.iter().find(|(path, s, _)| path == folder && *s == sig)
    {
        return Ok(LoadedFont::Glyphs(std::sync::Arc::clone(font)));
    }
    let read = |name: &str, limit: usize| -> Result<String, EngineError> {
        let path = folder.join(name);
        let bytes = read_limited(&path.display().to_string(), limit)?;
        String::from_utf8(bytes).map_err(|_| EngineError::BadParams(format!("{} is not text", path.display())))
    };
    let info = read("font.json", laserfoundry_text::MAX_FONT_JSON_BYTES)?;
    let font = if inkstitch {
        let svg = read("ltr.svg", laserfoundry_text::MAX_INKSTITCH_SVG_BYTES)?;
        laserfoundry_text::GlyphFont::parse_inkstitch(&info, &svg)
    } else {
        let glyphs = read("glyphs.stitchfoundry", laserfoundry_text::MAX_GLYPH_FILE_BYTES)?;
        laserfoundry_text::GlyphFont::parse(&info, &glyphs)
    }
    .map_err(|e| EngineError::BadParams(e.to_string()))?;
    let font = std::sync::Arc::new(font);
    if let Ok(mut cache) = font_cache().lock() {
        cache.retain(|(path, _, _)| path != folder);
        if cache.len() >= CACHED_FONTS {
            cache.remove(0);
        }
        cache.push((folder.to_path_buf(), sig, std::sync::Arc::clone(&font)));
    }
    Ok(LoadedFont::Glyphs(font))
}

fn read_font_file(path: &str) -> Result<Vec<u8>, EngineError> {
    read_limited(path, laserfoundry_text::MAX_FONT_BYTES)
}

/// Reads a file of at most `limit` bytes; a larger one is an error, not a truncation.
fn read_limited(path: &str, limit: usize) -> Result<Vec<u8>, EngineError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| EngineError::BadParams(format!("cannot open the font {path}: {e}")))?;
    let mut data = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut data).map_err(|e| EngineError::BadParams(format!("cannot read the font {path}: {e}")))?;
    if data.len() > limit {
        return Err(EngineError::BadParams("the font file is too large".into()));
    }
    Ok(data)
}

#[derive(Deserialize)]
struct OnPathParams {
    text: ShapeId,
    path: ShapeId,
    #[serde(default)]
    offset: f64,
    #[serde(default = "yes")]
    keep: bool,
}

fn yes() -> bool {
    true
}

/// Most points the bent text may have.
const MAX_BENT_POINTS: usize = 2_000_000;

fn on_path(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: OnPathParams = parse(params)?;
    finite(&[p.offset])?;
    if p.text == p.path {
        return Err(EngineError::BadParams("the text and the path must be different shapes".into()));
    }
    let text = s.doc.shape(p.text).ok_or_else(|| EngineError::NotFound(format!("shape {} does not exist", p.text)))?;
    let ShapeKind::Text { style, path, .. } = &text.kind else {
        return Err(EngineError::BadParams(format!("shape {} is not text", p.text)));
    };
    let guide = s.doc.shape(p.path).ok_or_else(|| EngineError::NotFound(format!("shape {} does not exist", p.path)))?;
    // The longest line of the guide shape, in workspace mm.
    let line = guide
        .world_polylines()
        .into_iter()
        .max_by(|a, b| a.length().total_cmp(&b.length()))
        .ok_or_else(|| EngineError::BadParams("the path has no geometry".into()))?;
    let along = laserfoundry_geom::along::Along::new(&line).ok_or_else(|| EngineError::BadParams("the path is too short to follow".into()))?;

    // Letters keep their own size (the text shape's scale) and their baseline sits on the line.
    let m = text.transform.as_coeffs();
    let (sx, sy) = (m[0].hypot(m[1]), m[2].hypot(m[3]));
    let baseline = style.height;
    let layer = text.layer;
    let mut bent = laserfoundry_geom::BezPath::new();
    let mut count = 0usize;
    for pl in laserfoundry_geom::flatten(path, BEND_TOLERANCE_MM) {
        count += pl.points.len();
        if count > MAX_BENT_POINTS {
            return Err(EngineError::BadParams("that text is too large to bend".into()));
        }
        // Long straight edges are cut into short steps so they can follow a curve.
        let mut dense: Vec<laserfoundry_geom::Point> = Vec::with_capacity(pl.points.len());
        let n = pl.points.len();
        let edges = if pl.closed { n } else { n.saturating_sub(1) };
        for i in 0..edges {
            let (a, b) = (pl.points[i], pl.points[(i + 1) % n]);
            let steps = ((a.distance(b) * sx.max(sy) / BEND_STEP_MM).ceil() as usize).clamp(1, 10_000);
            count += steps;
            if count > MAX_BENT_POINTS {
                return Err(EngineError::BadParams("that text is too large to bend".into()));
            }
            dense.extend((0..steps).map(|k| a.lerp(b, k as f64 / steps as f64)));
        }
        if !pl.closed
            && let Some(last) = pl.points.last()
        {
            dense.push(*last);
        }
        for (i, pt) in dense.iter().enumerate() {
            let (at, dir) = along.at(p.offset + pt.x * sx);
            let up = laserfoundry_geom::Vec2::new(dir.y, -dir.x);
            let q = at + up * ((baseline - pt.y) * sy);
            if i == 0 {
                bent.move_to(q);
            } else {
                bent.line_to(q);
            }
        }
        if pl.closed {
            bent.close_path();
        }
    }
    let new_id = s.doc.add_shape(Shape::path("Text on path", layer, bent));
    if !p.keep {
        s.doc.shapes.retain(|x| x.id != p.text);
    }
    s.selection = vec![new_id];
    Ok(json!({ "id": new_id }))
}

/// Flattening tolerance used before bending, mm: curves are straightened finely enough that the
/// bent result stays smooth.
const BEND_TOLERANCE_MM: f64 = 0.02;
/// Longest straight step kept when bending, mm.
const BEND_STEP_MM: f64 = 0.5;
