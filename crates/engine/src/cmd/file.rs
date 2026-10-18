use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{DynamicImage, ImageReader, Limits};
use lightfoundry_doc::{Document, LAYER_COLORS, LayerId, RasterImage, Shape, ShapeId, ShapeKind};
use lightfoundry_geom::Affine;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, parse, shapes::finite};
use crate::{EngineError, Session};

/// Largest decoded image we accept, in pixels.
const MAX_IMAGE_PIXELS: u64 = 50_000_000;
/// Default engraving resolution for imported images: 254 dpi = 0.1 mm per pixel.
const DEFAULT_MM_PER_PX: f64 = 0.1;

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec { id: "file.new", label: "New", menu: "File", shortcut: Some("Ctrl+N"), params_doc: "{}", mutates: false, run: new },
    CommandSpec {
        id: "file.open",
        label: "Open…",
        menu: "File",
        shortcut: Some("Ctrl+O"),
        params_doc: "{path | data_base64, template?: bool}  — template opens it as a new untitled project; data_base64 is the contents of a .lightfoundry file",
        mutates: false,
        run: open,
    },
    CommandSpec {
        id: "file.project_data",
        label: "Project as Data",
        menu: "",
        shortcut: None,
        params_doc: "{}  — returns the project file's contents as data_base64 and marks the document saved (for hosts that download it, like the browser)",
        mutates: false,
        run: project_data,
    },
    CommandSpec {
        id: "file.save",
        label: "Save",
        menu: "File",
        shortcut: Some("Ctrl+S"),
        params_doc: "{path?}  (default: the current file)",
        mutates: false,
        run: save,
    },
    CommandSpec {
        id: "file.import_svg",
        label: "Import SVG…",
        menu: "File/Import",
        shortcut: None,
        params_doc: "{path | data, x?, y?, layer_by_color?: true}",
        mutates: true,
        run: import_svg,
    },
    CommandSpec {
        id: "file.import_dxf",
        label: "Import DXF…",
        menu: "File/Import",
        shortcut: None,
        params_doc: "{path | data, x?, y?, join?: true}  — join chains loose line segments into paths",
        mutates: true,
        run: import_dxf,
    },
    CommandSpec {
        id: "file.import_image",
        label: "Import Image…",
        menu: "File/Import",
        shortcut: None,
        params_doc: "{path | data_base64 (+ name?), x?, y?, width? (mm), layer?}",
        mutates: true,
        run: import_image,
    },
    CommandSpec {
        id: "file.autosave",
        label: "Autosave",
        menu: "",
        shortcut: None,
        params_doc: "{path}  — writes a recovery copy; does not change the document's path or unsaved state",
        mutates: false,
        run: autosave,
    },
    CommandSpec {
        id: "file.recover",
        label: "Recover",
        menu: "",
        shortcut: None,
        params_doc: "{path}  — opens a recovery copy as an unsaved, untitled document",
        mutates: false,
        run: recover,
    },
    CommandSpec {
        id: "file.export_svg",
        label: "Save SVG…",
        menu: "File/Export",
        shortcut: None,
        params_doc: "{path?, include? (return the SVG text), selection_only?, include_hidden? (layers with output off)}  — millimetre units, one group per layer coloured like the layer (lines as hairline strokes, fills as fills), pictures embedded as PNG with their tone adjustments applied",
        mutates: false,
        run: export_svg,
    },
    CommandSpec {
        id: "file.export_gcode",
        label: "Save G-code…",
        menu: "File/Export",
        shortcut: None,
        params_doc: "{path}",
        mutates: false,
        run: export_gcode,
    },
];

fn new(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    let machine = s.doc.machine.clone();
    s.replace_document(Document { machine, ..Document::default() }, None);
    Ok(json!({}))
}

#[derive(Deserialize)]
struct PathParams {
    path: PathBuf,
}

/// Largest image file read, in bytes.
const MAX_IMAGE_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
struct OpenParams {
    path: Option<PathBuf>,
    data_base64: Option<String>,
    /// Open as a new untitled project (the file is a template and is never saved over).
    #[serde(default)]
    template: bool,
}

fn open(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: OpenParams = parse(params)?;
    let (doc, path) = match (p.path, p.data_base64) {
        (Some(path), _) => (lightfoundry_format::load(&path)?, Some(path)),
        (None, Some(data)) => (lightfoundry_format::from_bytes(&decode_data(&data)?)?, None),
        (None, None) => return Err(EngineError::BadParams("give either path or data_base64".into())),
    };
    let shapes = doc.shapes.len();
    s.replace_document(doc, if p.template { None } else { path });
    if p.template {
        s.dirty = true; // a new project made from a template has never been saved
    }
    Ok(json!({ "shapes": shapes }))
}

fn project_data(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    let bytes = lightfoundry_format::to_bytes(&s.doc)?;
    s.dirty = false;
    Ok(json!({ "data_base64": lightfoundry_doc::base64::encode(&bytes), "bytes": bytes.len() }))
}

pub(super) fn decode_data(data: &str) -> Result<Vec<u8>, EngineError> {
    lightfoundry_doc::base64::decode(data).ok_or_else(|| EngineError::BadParams("data_base64 is not valid base64".into()))
}

#[derive(Deserialize, Default)]
struct OptionalPath {
    path: Option<PathBuf>,
}

fn save(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: OptionalPath = parse(params)?;
    let path =
        p.path.or_else(|| s.path.clone()).ok_or_else(|| EngineError::BadParams("no path given and the document has never been saved".into()))?;
    lightfoundry_format::save(&path, &s.doc)?;
    s.path = Some(path.clone());
    s.dirty = false;
    Ok(json!({ "path": path }))
}

#[derive(Deserialize)]
struct ImportParams {
    path: Option<PathBuf>,
    data: Option<String>,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default = "yes")]
    layer_by_color: bool,
    /// DXF only: chain loose line segments into paths (default true).
    #[serde(default = "yes")]
    join: bool,
}

fn yes() -> bool {
    true
}

fn import_svg(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: ImportParams = parse(params)?;
    finite(&[p.x, p.y])?;
    let bytes = source_bytes(&p)?;
    let svg = lightfoundry_svg::import_svg(&bytes).map_err(|e| EngineError::Import(e.to_string()))?;
    let mut ids = Vec::new();
    for ip in svg.paths {
        let layer = if p.layer_by_color { ip.color.as_deref().map_or(0, layer_for_color) } else { 0 };
        let mut shape = Shape::path(ip.name, layer, ip.path);
        shape.transform = Affine::translate((p.x, p.y));
        ids.push(s.doc.add_shape(shape));
    }
    // Pictures embedded in the SVG; one that cannot be decoded is reported, not fatal.
    let mut skipped_images = 0;
    for img in svg.images {
        let Ok(raster) = decode_picture(&img.bytes) else {
            skipped_images += 1;
            continue;
        };
        let shape = Shape {
            id: 0,
            name: img.name,
            layer: 0,
            transform: Affine::translate((p.x, p.y)) * img.transform,
            kind: ShapeKind::Image { image: Arc::new(raster), width_mm: img.width_mm, height_mm: img.height_mm, adjust: Default::default() },
            cut: None,
            group: None,
            order: 0,
        };
        ids.push(s.doc.add_shape(shape));
    }
    s.selection = ids.clone();
    Ok(json!({ "ids": ids, "width_mm": svg.width_mm, "height_mm": svg.height_mm, "skipped_images": skipped_images }))
}

/// Chains the open shapes among `ids` (per layer) into longer paths, replacing them in `ids`.
/// Shapes that are already closed are left exactly as imported (curves stay curves).
fn join_loose_paths(s: &mut Session, ids: &mut Vec<ShapeId>) -> Result<(), EngineError> {
    let mut by_layer: Vec<(LayerId, Vec<ShapeId>, Vec<lightfoundry_geom::Polyline>)> = Vec::new();
    for id in ids.iter() {
        let Some(shape) = s.doc.shape(*id) else { continue };
        let open: Vec<_> = shape.world_polylines().into_iter().filter(|l| !l.closed).collect();
        if open.is_empty() || shape.world_polylines().iter().any(|l| l.closed) {
            continue;
        }
        match by_layer.iter_mut().find(|(l, _, _)| *l == shape.layer) {
            Some((_, v, lines)) => {
                v.push(*id);
                lines.extend(open);
            }
            None => by_layer.push((shape.layer, vec![*id], open)),
        }
    }
    for (layer, sources, lines) in by_layer {
        if sources.len() < 2 {
            continue;
        }
        let chains =
            lightfoundry_pathops::join(&lines, super::pathops::DEFAULT_JOIN_TOLERANCE / 5.0).map_err(|e| EngineError::Import(e.to_string()))?;
        s.doc.remove_shapes(&sources);
        ids.retain(|i| !sources.contains(i));
        for chain in chains {
            let name = if chain.closed { "Joined shape" } else { "Joined path" };
            ids.push(s.doc.add_shape(Shape::path(name, layer, super::pathops::polylines_to_path(std::slice::from_ref(&chain)))));
        }
    }
    Ok(())
}

fn import_dxf(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: ImportParams = parse(params)?;
    finite(&[p.x, p.y])?;
    let bytes = source_bytes(&p)?;
    let dxf = lightfoundry_dxf::import_dxf(&bytes).map_err(|e| EngineError::Import(e.to_string()))?;
    // DXF layer names map to cut layers in order of first appearance.
    let mut names: Vec<String> = Vec::new();
    let mut ids = Vec::new();
    let translate = Affine::translate((p.x, p.y));
    for dp in dxf.paths {
        let idx = names.iter().position(|n| *n == dp.layer).unwrap_or_else(|| {
            names.push(dp.layer.clone());
            names.len() - 1
        });
        let layer = (idx % LAYER_COLORS.len()) as LayerId;
        let mut shape = Shape::path(dp.layer, layer, dp.path);
        shape.transform = translate;
        // Loose segments are joined below; keep a handle to them until then.
        ids.push(s.doc.add_shape(shape));
    }
    if p.join {
        join_loose_paths(s, &mut ids)?;
    }
    s.selection = ids.clone();
    Ok(json!({ "ids": ids, "width_mm": dxf.width_mm, "height_mm": dxf.height_mm, "skipped": dxf.skipped }))
}

#[derive(Deserialize)]
struct ImageParams {
    path: Option<PathBuf>,
    data_base64: Option<String>,
    name: Option<String>,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    width: Option<f64>,
    #[serde(default)]
    layer: LayerId,
}

fn import_image(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: ImageParams = parse(params)?;
    finite(&[p.x, p.y])?;
    let (mut reader, name) = match (&p.path, &p.data_base64) {
        (Some(path), _) => {
            let Some(bytes) = lightfoundry_format::read_capped(std::path::Path::new(path), MAX_IMAGE_FILE_BYTES)? else {
                return Err(EngineError::Import("the image file is too large".into()));
            };
            (ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?, file_name(path))
        }
        (None, Some(data)) => {
            if data.len() > MAX_IMAGE_FILE_BYTES as usize * 4 / 3 + 16 {
                return Err(EngineError::Import("the image file is too large".into()));
            }
            let bytes = decode_data(data)?;
            let name = p.name.clone().map_or_else(|| "image".to_string(), |n| n.chars().take(80).collect());
            (ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?, name)
        }
        (None, None) => return Err(EngineError::BadParams("give either path or data_base64".into())),
    };
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 8);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| EngineError::Import(e.to_string()))?;
    let raster = to_gray(&decoded)?;
    let aspect = f64::from(raster.height) / f64::from(raster.width);
    let width_mm = match p.width {
        Some(w) if w.is_finite() && w > 0.0 && w < 1e5 => w,
        Some(_) => return Err(EngineError::BadParams("width must be positive".into())),
        None => f64::from(raster.width) * DEFAULT_MM_PER_PX,
    };
    let shape = Shape {
        id: 0,
        name,
        layer: p.layer,
        transform: Affine::translate((p.x, p.y)),
        kind: ShapeKind::Image { image: Arc::new(raster), width_mm, height_mm: width_mm * aspect, adjust: Default::default() },
        cut: None,
        group: None,
        order: 0,
    };
    let id = s.doc.add_shape(shape);
    s.selection = vec![id];
    Ok(json!({ "id": id, "width_mm": width_mm, "height_mm": width_mm * aspect }))
}

/// Decodes picture bytes (format guessed) to the gray raster the engine works with.
fn decode_picture(bytes: &[u8]) -> Result<RasterImage, EngineError> {
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 8);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| EngineError::Import(e.to_string()))?;
    to_gray(&decoded)
}

fn autosave(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: PathParams = parse(params)?;
    lightfoundry_format::save(&p.path, &s.doc)?;
    Ok(json!({ "path": p.path, "revision": s.revision }))
}

fn recover(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: PathParams = parse(params)?;
    let doc = lightfoundry_format::load(&p.path)?;
    let shapes = doc.shapes.len();
    s.replace_document(doc, None);
    // Recovered work was never saved: keep it marked as changed so closing asks first.
    s.dirty = true;
    Ok(json!({ "shapes": shapes }))
}

#[derive(Deserialize)]
struct ExportSvgParams {
    path: Option<PathBuf>,
    #[serde(default)]
    include: bool,
    #[serde(default)]
    selection_only: bool,
    #[serde(default)]
    include_hidden: bool,
}

fn export_svg(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: ExportSvgParams = parse(params)?;
    if p.path.is_none() && !p.include {
        return Err(EngineError::BadParams("give a path, or include: true to get the text".into()));
    }
    let only = p.selection_only.then(|| s.selection.clone());
    let (svg, shapes) = super::svg_export::export(&s.doc, only.as_deref(), p.include_hidden)?;
    if let Some(path) = &p.path {
        std::fs::write(path, &svg)?;
    }
    Ok(if p.include { json!({ "shapes": shapes, "bytes": svg.len(), "svg": svg }) } else { json!({ "shapes": shapes, "bytes": svg.len() }) })
}

fn export_gcode(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: PathParams = parse(params)?;
    let program = super::job::generate(s)?;
    std::fs::write(&p.path, &program)?;
    Ok(json!({ "path": p.path, "bytes": program.len() }))
}

fn source_bytes(p: &ImportParams) -> Result<Vec<u8>, EngineError> {
    match (&p.path, &p.data) {
        (Some(path), _) => lightfoundry_format::read_capped(std::path::Path::new(path), MAX_IMAGE_FILE_BYTES)?
            .ok_or_else(|| EngineError::Import("the file is too large".into())),
        (None, Some(data)) => Ok(data.clone().into_bytes()),
        (None, None) => Err(EngineError::BadParams("give either path or data".into())),
    }
}

/// Exact colour match against the layer palette; anything else lands on layer 0.
pub(super) fn layer_for_color(hex: &str) -> LayerId {
    LAYER_COLORS.iter().position(|c| c.eq_ignore_ascii_case(hex)).map_or(0, |i| i as LayerId)
}

fn to_gray(img: &DynamicImage) -> Result<RasterImage, EngineError> {
    let la = img.to_luma_alpha8();
    let (w, h) = la.dimensions();
    if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
        return Err(EngineError::Import("image is too large".into()));
    }
    // Composite over white: transparent pixels must not burn.
    let over_white = |value: u8, alpha: u8| ((u32::from(value) * u32::from(alpha) + 255 * (255 - u32::from(alpha))) / 255) as u8;
    let gray = la.pixels().map(|px| over_white(px[0], px[1])).collect();
    let image = RasterImage::new(w, h, gray).ok_or_else(|| EngineError::Import("empty image".into()))?;
    let rgba = img.to_rgba8();
    let rgb = rgba.pixels().flat_map(|px| [over_white(px[0], px[3]), over_white(px[1], px[3]), over_white(px[2], px[3])]).collect();
    // Grey pictures carry no colour worth keeping.
    let coloured = rgba.pixels().any(|px| px[0] != px[1] || px[1] != px[2]);
    Ok(if coloured { image.clone().with_rgb(rgb).unwrap_or(image) } else { image })
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into())
}
