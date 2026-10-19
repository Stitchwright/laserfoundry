//! The LaserFoundry document model: cut layers, shapes, images and the machine profile.
//! Pure data + serde; no I/O, no UI.

use std::sync::Arc;

use laserfoundry_geom::{Affine, BezPath, Polyline, Rect, bounds_of_polylines, flatten};
use serde::{Deserialize, Serialize};

pub const LAYER_COLORS: [&str; 16] = [
    "#000000", "#0000ff", "#ff0000", "#00e000", "#d0d000", "#ff8000", "#00e0e0", "#ff00ff", "#b4b4b4", "#0000a0", "#a00000", "#00a000", "#a0a000",
    "#c08000", "#00a0ff", "#a000a0",
];

/// Flattening tolerance (mm) used for planning and previews.
pub const FLATTEN_TOLERANCE_MM: f64 = 0.02;

pub type ShapeId = u64;
pub type LayerId = u32;
pub type GroupId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub layers: Vec<Layer>,
    pub shapes: Vec<Shape>,
    pub machine: Machine,
    /// Where the job lands relative to the machine; absent in older files.
    #[serde(default)]
    pub job_origin: JobOrigin,
    /// How outlines are ordered and cleaned before cutting; absent in older files.
    #[serde(default)]
    pub optimize: Optimize,
    /// Named sets of cut settings the person saved (e.g. "3 mm birch ply, cut"); absent in older files.
    #[serde(default)]
    pub presets: Vec<Preset>,
    pub next_id: ShapeId,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            layers: (0..LAYER_COLORS.len() as u32).map(Layer::numbered).collect(),
            shapes: Vec::new(),
            machine: Machine::default(),
            job_origin: JobOrigin::default(),
            optimize: Optimize::default(),
            presets: Vec::new(),
            next_id: 1,
        }
    }
}

impl Document {
    /// A rough count of the memory the document's shapes use (picture pixels are shared between copies of a
    /// document and not counted). Used to bound how much undo history is kept.
    pub fn approx_bytes(&self) -> usize {
        self.shapes
            .iter()
            .map(|s| {
                let own = std::mem::size_of::<Shape>() + s.name.len();
                own + match &s.kind {
                    ShapeKind::Path { path } => std::mem::size_of_val(path.elements()),
                    ShapeKind::Text { content, path, .. } => content.len() + std::mem::size_of_val(path.elements()),
                    _ => 0,
                }
            })
            .sum()
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn shape(&self, id: ShapeId) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id == id)
    }

    pub fn shape_mut(&mut self, id: ShapeId) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id == id)
    }

    /// Adds a shape, assigning it a fresh id. Unknown layers fall back to layer 0.
    pub fn add_shape(&mut self, mut shape: Shape) -> ShapeId {
        shape.id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        if self.layer(shape.layer).is_none() {
            shape.layer = 0;
        }
        let id = shape.id;
        self.shapes.push(shape);
        id
    }

    /// `ids` plus every other member of any group they belong to, in document order.
    pub fn expand_groups(&self, ids: &[ShapeId]) -> Vec<ShapeId> {
        let groups: Vec<GroupId> = ids.iter().filter_map(|id| self.shape(*id)).filter_map(|s| s.group).collect();
        self.shapes.iter().filter(|s| ids.contains(&s.id) || s.group.is_some_and(|g| groups.contains(&g))).map(|s| s.id).collect()
    }

    /// Copies `ids` with `m` applied to each transform. Copies of a group form a new group of their
    /// own (never sharing the original's), so moving a copy does not drag the original along.
    pub fn duplicate_shapes(&mut self, ids: &[ShapeId], m: Affine) -> Vec<ShapeId> {
        let mut remap: Vec<(GroupId, GroupId)> = Vec::new();
        let mut out = Vec::new();
        for id in ids {
            let Some(mut copy) = self.shape(*id).cloned() else { continue };
            copy.transform = m * copy.transform;
            if let Some(old) = copy.group {
                let new = match remap.iter().find(|(o, _)| *o == old) {
                    Some((_, n)) => *n,
                    None => {
                        let n = self.next_id;
                        self.next_id = self.next_id.saturating_add(1);
                        remap.push((old, n));
                        n
                    }
                };
                copy.group = Some(new);
            }
            out.push(self.add_shape(copy));
        }
        out
    }

    pub fn remove_shapes(&mut self, ids: &[ShapeId]) {
        self.shapes.retain(|s| !ids.contains(&s.id));
    }

    /// Bounds of the given shapes in workspace mm.
    pub fn bounds_of(&self, ids: &[ShapeId]) -> Option<Rect> {
        ids.iter().filter_map(|id| self.shape(*id)).filter_map(Shape::world_bounds).reduce(|a, b| a.union(b))
    }
}

/// Most presets kept in a document.
pub const MAX_PRESETS: usize = 200;

/// A named snapshot of a layer's cut settings. Identity (id, name, colour) and whether the layer
/// outputs are not part of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub settings: Layer,
}

impl Layer {
    /// Copies the cut settings of `preset` onto this layer, keeping its identity and output flag.
    pub fn apply_settings(&mut self, preset: &Layer) {
        let keep = Layer {
            id: self.id,
            name: std::mem::take(&mut self.name),
            color: std::mem::take(&mut self.color),
            output: self.output,
            ..preset.clone()
        };
        *self = keep;
    }
}

/// Cut-order options for outlines.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Optimize {
    /// Shapes nested inside others are cut first, so parts do not drop out before their holes.
    pub inner_first: bool,
    /// Pick the next outline nearest to the head instead of following the order shapes were drawn in.
    pub reduce_travel: bool,
    /// Drop outlines that lie exactly on top of an earlier one (they would be burned twice).
    pub remove_duplicates: bool,
    /// Finish each group of shapes before starting the next (shapes grouped with Ctrl+G stay together).
    #[serde(default)]
    pub by_group: bool,
    /// Where the head is taken to be when ordering starts, workspace mm (`None` = the bed's origin).
    #[serde(default)]
    pub start: Option<[f64; 2]>,
}

impl Default for Optimize {
    fn default() -> Self {
        Optimize { inner_first: true, reduce_travel: true, remove_duplicates: true, by_group: false, start: None }
    }
}

/// How the job is positioned on the machine.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct JobOrigin {
    pub start_from: StartFrom,
    /// Which point of the job's bounds sits at the origin (relative modes only).
    pub anchor: Anchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartFrom {
    /// Design coordinates are machine coordinates: the bed in the editor is the bed on the machine.
    #[default]
    Absolute,
    /// The user has set the work-coordinate zero on the machine; the job anchor goes there.
    UserOrigin,
    /// The head's current position becomes zero when the job starts.
    CurrentPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// A plain-words name (for tooltips and screen readers).
    pub fn label(self) -> &'static str {
        match self {
            Anchor::TopLeft => "top left",
            Anchor::Top => "top",
            Anchor::TopRight => "top right",
            Anchor::Left => "left",
            Anchor::Center => "center",
            Anchor::Right => "right",
            Anchor::BottomLeft => "bottom left",
            Anchor::Bottom => "bottom",
            Anchor::BottomRight => "bottom right",
        }
    }

    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    /// The anchor's point on `b` (workspace mm, y down).
    pub fn point(self, b: Rect) -> laserfoundry_geom::Point {
        let x = match self {
            Anchor::TopLeft | Anchor::Left | Anchor::BottomLeft => b.x0,
            Anchor::Top | Anchor::Center | Anchor::Bottom => (b.x0 + b.x1) / 2.0,
            Anchor::TopRight | Anchor::Right | Anchor::BottomRight => b.x1,
        };
        let y = match self {
            Anchor::TopLeft | Anchor::Top | Anchor::TopRight => b.y0,
            Anchor::Left | Anchor::Center | Anchor::Right => (b.y0 + b.y1) / 2.0,
            Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => b.y1,
        };
        laserfoundry_geom::Point::new(x, y)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub color: String,
    pub mode: LayerMode,
    /// mm/min
    pub speed: f64,
    /// Percent of the machine's S range, 0..=100.
    pub min_power: f64,
    pub max_power: f64,
    pub passes: u32,
    /// Fill line spacing in mm.
    pub interval: f64,
    /// Fill scan angle in degrees.
    pub scan_angle: f64,
    /// Kerf compensation for outlines, mm. Closed outlines are offset by this distance before
    /// cutting: positive grows the outer outline and shrinks holes (the path sits outside a part
    /// and inside a hole, so both come out the right size once the beam has taken its width).
    #[serde(default)]
    pub kerf_offset: f64,
    /// Tabs per outer outline (0 = none): short uncut gaps that keep a part attached to the sheet.
    #[serde(default)]
    pub tab_count: u32,
    /// Width of each tab, mm.
    #[serde(default = "default_tab_width")]
    pub tab_width: f64,
    /// Length of the approach line before a closed outline, on the scrap side, mm.
    #[serde(default)]
    pub lead_in: f64,
    /// Length of the exit line after a closed outline, on the scrap side, mm.
    #[serde(default)]
    pub lead_out: f64,
    /// Perforation: cut this many mm, then skip `perf_gap` mm, along outlines (both 0 = solid line).
    #[serde(default)]
    pub perf_cut: f64,
    #[serde(default)]
    pub perf_gap: f64,
    /// Fill twice, the second time at right angles to the first.
    #[serde(default)]
    pub crosshatch: bool,
    /// Run-up and run-out beyond the ends of fill lines and picture rows, mm, with the laser off so
    /// the head is at speed when it starts burning. `None` uses the machine's setting for pictures
    /// and none for fills.
    #[serde(default)]
    pub overscan: Option<f64>,
    /// Outlines start at the layer's minimum power and rise to full power over this many mm (0 = off),
    /// which avoids a burn spot where the beam first pierces.
    #[serde(default)]
    pub ramp_length: f64,
    /// Z axis height for this layer, mm from the work zero (0 and no step = Z is not touched).
    #[serde(default)]
    pub z_offset: f64,
    /// Extra Z movement for each pass after the first, mm (negative lowers a head that moves down
    /// with negative Z, e.g. to follow a cut deeper).
    #[serde(default)]
    pub z_per_pass: f64,
    pub bidirectional: bool,
    pub dither: Dither,
    pub output: bool,
    pub air_assist: bool,
}

fn default_tab_width() -> f64 {
    1.0
}

impl Layer {
    pub fn numbered(id: LayerId) -> Layer {
        Layer {
            id,
            name: format!("C{id:02}"),
            color: LAYER_COLORS.get(id as usize % LAYER_COLORS.len()).copied().unwrap_or("#000000").to_string(),
            mode: LayerMode::Line,
            speed: 1000.0,
            min_power: 10.0,
            max_power: 60.0,
            passes: 1,
            interval: 0.1,
            scan_angle: 0.0,
            kerf_offset: 0.0,
            tab_count: 0,
            tab_width: default_tab_width(),
            lead_in: 0.0,
            lead_out: 0.0,
            perf_cut: 0.0,
            perf_gap: 0.0,
            crosshatch: false,
            overscan: None,
            ramp_length: 0.0,
            z_offset: 0.0,
            z_per_pass: 0.0,
            bidirectional: true,
            dither: Dither::FloydSteinberg,
            output: true,
            air_assist: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerMode {
    Line,
    Fill,
    FillAndLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dither {
    Threshold,
    FloydSteinberg,
    Grayscale,
    /// Error diffusion that spreads only part of the error: crisp highlights, good for line art.
    Atkinson,
    /// Wide error diffusion: smoother gradients, slower.
    Jarvis,
    /// 8x8 Bayer matrix: a regular, repeatable pattern.
    Ordered,
    /// Round dots on a 45 degree screen whose size follows the darkness (newsprint look).
    Halftone,
}

impl Dither {
    pub const ALL: [Dither; 7] =
        [Dither::Threshold, Dither::FloydSteinberg, Dither::Atkinson, Dither::Jarvis, Dither::Ordered, Dither::Halftone, Dither::Grayscale];

    pub fn label(self) -> &'static str {
        match self {
            Dither::Threshold => "Threshold",
            Dither::FloydSteinberg => "Floyd–Steinberg",
            Dither::Grayscale => "Grayscale",
            Dither::Atkinson => "Atkinson",
            Dither::Jarvis => "Jarvis",
            Dither::Ordered => "Ordered (Bayer)",
            Dither::Halftone => "Halftone",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: ShapeId,
    pub name: String,
    pub layer: LayerId,
    /// Maps local geometry to workspace mm (y down, origin top-left of the bed).
    pub transform: Affine,
    pub kind: ShapeKind,
    /// Settings that replace the layer's for this shape only (test grids, one-off tweaks).
    #[serde(default)]
    pub cut: Option<CutOverride>,
    /// Shapes sharing a group id are selected and moved together.
    #[serde(default)]
    pub group: Option<GroupId>,
    /// Cut priority within its layer: lower numbers are cut first, ties keep the drawing order.
    #[serde(default)]
    pub order: i32,
}

/// Per-shape replacement for some of its layer's cut settings; `None` fields keep the layer's value.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CutOverride {
    /// mm/min
    pub speed: Option<f64>,
    /// Percent
    pub min_power: Option<f64>,
    pub max_power: Option<f64>,
    pub passes: Option<u32>,
}

impl CutOverride {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The layer as this override changes it.
    pub fn apply(&self, layer: &Layer) -> Layer {
        let mut l = layer.clone();
        l.speed = self.speed.unwrap_or(l.speed);
        l.min_power = self.min_power.unwrap_or(l.min_power);
        l.max_power = self.max_power.unwrap_or(l.max_power);
        l.passes = self.passes.unwrap_or(l.passes);
        l
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShapeKind {
    Path {
        path: BezPath,
    },
    /// Editable text. `path` is the laid-out stroke geometry (regenerated by the engine whenever the
    /// content or style changes); the transform places and scales it like any other shape.
    Text {
        content: String,
        style: TextSettings,
        path: BezPath,
    },
    /// Raster occupying local `0..width_mm × 0..height_mm`.
    Image {
        image: Arc<RasterImage>,
        width_mm: f64,
        height_mm: f64,
        /// Tone adjustments applied before dithering; the stored pixels are never changed.
        #[serde(default)]
        adjust: ImageAdjust,
    },
}

/// Text style as stored in the document (mirrors the text crate's style).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSettings {
    /// Cap height in mm.
    pub height: f64,
    pub letter_spacing: f64,
    pub line_spacing: f64,
    pub align: TextAlign,
    /// Path of a TrueType/OpenType file to draw outlines from (filled text); `None` = the built-in
    /// single-line font. Only needed to edit the text again: the laid-out shapes are stored.
    #[serde(default)]
    pub font: Option<String>,
    /// Letters stacked top to bottom, one column per line, the first line rightmost.
    #[serde(default)]
    pub vertical: bool,
    /// A second font file that supplies characters `font` has no glyph for.
    #[serde(default)]
    pub fallback_font: Option<String>,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self { height: 10.0, letter_spacing: 1.0, line_spacing: 1.0, align: TextAlign::Left, font: None, vertical: false, fallback_font: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl Shape {
    /// The vector geometry of path and text shapes (local space); `None` for images.
    pub fn vector_path(&self) -> Option<&BezPath> {
        match &self.kind {
            ShapeKind::Path { path } | ShapeKind::Text { path, .. } => Some(path),
            ShapeKind::Image { .. } => None,
        }
    }

    pub fn path(name: impl Into<String>, layer: LayerId, path: BezPath) -> Shape {
        Shape { id: 0, name: name.into(), layer, transform: Affine::IDENTITY, kind: ShapeKind::Path { path }, cut: None, group: None, order: 0 }
    }

    /// Flattened outlines in workspace mm. Images yield their bounding rectangle.
    pub fn world_polylines(&self) -> Vec<Polyline> {
        match &self.kind {
            ShapeKind::Path { path } | ShapeKind::Text { path, .. } => {
                let mut p = path.clone();
                p.apply_affine(self.transform);
                flatten(&p, FLATTEN_TOLERANCE_MM)
            }
            ShapeKind::Image { width_mm, height_mm, .. } => {
                let p = laserfoundry_geom::rect_path(0.0, 0.0, *width_mm, *height_mm);
                let mut p = p;
                p.apply_affine(self.transform);
                flatten(&p, FLATTEN_TOLERANCE_MM)
            }
        }
    }

    pub fn world_bounds(&self) -> Option<Rect> {
        bounds_of_polylines(&self.world_polylines())
    }
}

/// Tone adjustments for an image, applied to a gray value before it is dithered.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ImageAdjust {
    /// -1..=1, added to the normalised value.
    pub brightness: f64,
    /// -1..=1; positive stretches around mid-gray, negative flattens.
    pub contrast: f64,
    /// 0.1..=5; above 1 brightens midtones.
    pub gamma: f64,
    pub negative: bool,
}

impl Default for ImageAdjust {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 0.0, gamma: 1.0, negative: false }
    }
}

impl ImageAdjust {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Maps a gray value 0..=255 to its adjusted value; settings and input are clamped, so the
    /// result is always within 0..=255.
    pub fn apply(&self, value: f32) -> f32 {
        let finite = |v: f64, fallback: f64| if v.is_finite() { v } else { fallback };
        let mut x = (f64::from(value) / 255.0).clamp(0.0, 1.0);
        if !x.is_finite() {
            x = 1.0;
        }
        x += finite(self.brightness, 0.0).clamp(-1.0, 1.0);
        let c = finite(self.contrast, 0.0).clamp(-1.0, 1.0);
        let factor = if c >= 0.0 { 1.0 / (1.0 - c).max(0.01) } else { 1.0 + c };
        x = ((x - 0.5) * factor + 0.5).clamp(0.0, 1.0);
        x = x.powf(1.0 / finite(self.gamma, 1.0).clamp(0.1, 5.0));
        if self.negative {
            x = 1.0 - x;
        }
        (x.clamp(0.0, 1.0) * 255.0) as f32
    }
}

/// 8-bit grayscale raster, 255 = white (no burn).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    #[serde(with = "base64_bytes")]
    pub gray: Vec<u8>,
    /// The picture's colours, three bytes (R, G, B) per pixel, when it has any. `gray` is what is
    /// burned; this is kept for colour-aware tracing. Absent in older files and in grey pictures.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "base64_opt")]
    pub rgb: Option<Vec<u8>>,
}

impl RasterImage {
    /// Returns `None` when the buffer length does not match the dimensions.
    pub fn new(width: u32, height: u32, gray: Vec<u8>) -> Option<RasterImage> {
        let expected = (width as usize).checked_mul(height as usize)?;
        (expected == gray.len() && expected > 0).then_some(RasterImage { width, height, gray, rgb: None })
    }

    /// The same picture carrying its colours; `None` when `rgb` is not three bytes per pixel.
    pub fn with_rgb(mut self, rgb: Vec<u8>) -> Option<RasterImage> {
        (rgb.len() == self.gray.len().checked_mul(3)?).then(|| {
            self.rgb = Some(rgb);
            self
        })
    }

    /// The colour planes as grey pictures, so every grey operation can be applied to each.
    fn planes(&self) -> Option<[RasterImage; 3]> {
        let rgb = self.rgb.as_ref()?;
        let plane = |c: usize| RasterImage::new(self.width, self.height, rgb.as_chunks::<3>().0.iter().map(|px| px[c]).collect());
        Some([plane(0)?, plane(1)?, plane(2)?])
    }

    /// Applies a grey-only operation to the picture and to each colour plane, keeping them in step.
    fn on_all(&self, op: impl Fn(&RasterImage) -> Option<RasterImage>) -> Option<RasterImage> {
        let mut out = op(self)?;
        if let Some([r, g, b]) = self.planes() {
            let (r, g, b) = (op(&r)?, op(&g)?, op(&b)?);
            let rgb = r.gray.iter().zip(&g.gray).zip(&b.gray).flat_map(|((r, g), b)| [*r, *g, *b]).collect();
            out = out.with_rgb(rgb)?;
        }
        Some(out)
    }

    /// The pixels in columns `x0..x1` and rows `y0..y1`; `None` for an empty or out-of-range box.
    pub fn crop(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> Option<RasterImage> {
        self.on_all(|p| p.crop_gray(x0, y0, x1, y1))
    }

    fn crop_gray(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> Option<RasterImage> {
        if x0 >= x1 || y0 >= y1 || x1 > self.width || y1 > self.height {
            return None;
        }
        let w = self.width as usize;
        let mut gray = Vec::with_capacity((x1 - x0) as usize * (y1 - y0) as usize);
        for y in y0 as usize..y1 as usize {
            gray.extend_from_slice(self.gray.get(y * w + x0 as usize..y * w + x1 as usize)?);
        }
        RasterImage::new(x1 - x0, y1 - y0, gray)
    }

    /// The picture at `new_w` x `new_h` pixels: areas are averaged when shrinking and bilinearly
    /// interpolated when enlarging. `None` for an empty target or one with over 100 million pixels.
    pub fn resampled(&self, new_w: u32, new_h: u32) -> Option<RasterImage> {
        self.on_all(|p| p.resampled_gray(new_w, new_h))
    }

    fn resampled_gray(&self, new_w: u32, new_h: u32) -> Option<RasterImage> {
        const MAX_PIXELS: u64 = 100_000_000;
        if new_w == 0 || new_h == 0 || u64::from(new_w) * u64::from(new_h) > MAX_PIXELS {
            return None;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let (nw, nh) = (new_w as usize, new_h as usize);
        let px = |x: usize, y: usize| f32::from(self.gray.get(y.min(h - 1) * w + x.min(w - 1)).copied().unwrap_or(255));
        let mut out = Vec::with_capacity(nw * nh);
        for ny in 0..nh {
            // The source rows and columns this output pixel covers.
            let (y0, y1) = (ny as f64 * h as f64 / nh as f64, (ny + 1) as f64 * h as f64 / nh as f64);
            for nx in 0..nw {
                let (x0, x1) = (nx as f64 * w as f64 / nw as f64, (nx + 1) as f64 * w as f64 / nw as f64);
                let value = if x1 - x0 >= 1.0 || y1 - y0 >= 1.0 {
                    // Shrinking: average everything covered, weighted by overlap.
                    let (mut sum, mut weight) = (0.0f64, 0.0f64);
                    for sy in y0.floor() as usize..(y1.ceil() as usize).min(h) {
                        let wy = (y1.min(sy as f64 + 1.0) - y0.max(sy as f64)).max(0.0);
                        for sx in x0.floor() as usize..(x1.ceil() as usize).min(w) {
                            let wx = (x1.min(sx as f64 + 1.0) - x0.max(sx as f64)).max(0.0);
                            sum += f64::from(px(sx, sy)) * wx * wy;
                            weight += wx * wy;
                        }
                    }
                    if weight > 0.0 { (sum / weight) as f32 } else { 255.0 }
                } else {
                    // Enlarging: interpolate between the four nearest source pixels.
                    let (cx, cy) = (((x0 + x1) / 2.0 - 0.5).max(0.0), ((y0 + y1) / 2.0 - 0.5).max(0.0));
                    let (ix, iy) = (cx.floor() as usize, cy.floor() as usize);
                    let (fx, fy) = ((cx - ix as f64) as f32, (cy - iy as f64) as f32);
                    let top = px(ix, iy) * (1.0 - fx) + px(ix + 1, iy) * fx;
                    let bottom = px(ix, iy + 1) * (1.0 - fx) + px(ix + 1, iy + 1) * fx;
                    top * (1.0 - fy) + bottom * fy
                };
                out.push(value.round().clamp(0.0, 255.0) as u8);
            }
        }
        RasterImage::new(new_w, new_h, out)
    }

    /// Unsharp mask: adds `amount` times the difference from a 3x3 blur (edges are clamped), which
    /// makes outlines crisper before dithering. `amount` is limited to 0..=5.
    pub fn sharpened(&self, amount: f64) -> RasterImage {
        self.on_all(|p| Some(p.sharpened_gray(amount))).unwrap_or_else(|| self.sharpened_gray(amount))
    }

    fn sharpened_gray(&self, amount: f64) -> RasterImage {
        let amount = if amount.is_finite() { amount.clamp(0.0, 5.0) as f32 } else { 0.0 };
        let (w, h) = (self.width as usize, self.height as usize);
        let at = |x: isize, y: isize| -> f32 {
            let (x, y) = (x.clamp(0, w as isize - 1) as usize, y.clamp(0, h as isize - 1) as usize);
            f32::from(self.gray.get(y * w + x).copied().unwrap_or(255))
        };
        let mut out = Vec::with_capacity(self.gray.len());
        for y in 0..h as isize {
            for x in 0..w as isize {
                let mut sum = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        sum += at(x + dx, y + dy);
                    }
                }
                let centre = at(x, y);
                out.push((centre + amount * (centre - sum / 9.0)).round().clamp(0.0, 255.0) as u8);
            }
        }
        RasterImage { width: self.width, height: self.height, gray: out, rgb: None }
    }
}

/// Serde adapter storing byte buffers as standard base64 strings.
/// Base64 (standard alphabet, padded) for embedding binary data in JSON.
pub mod base64 {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// `None` for characters outside the alphabet. Padding and whitespace are ignored.
    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(text.len() / 4 * 3);
        let mut acc = 0u32;
        let mut bits = 0;
        for c in text.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()) {
            let v = ALPHABET.iter().position(|a| *a == c)? as u32;
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((acc >> bits) & 0xff) as u8);
            }
        }
        Some(out)
    }
}

mod base64_opt {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(bytes: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(b) => s.serialize_str(&super::base64::encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        match Option::<String>::deserialize(d)? {
            Some(text) => super::base64::decode(&text).map(Some).ok_or_else(|| D::Error::custom("invalid base64")),
            None => Ok(None),
        }
    }
}

mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::base64::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        super::base64::decode(&text).ok_or_else(|| D::Error::custom("invalid base64"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    pub bed_width: f64,
    pub bed_height: f64,
    pub origin: Origin,
    /// GRBL `$30` maximum spindle speed.
    pub max_s: u32,
    /// `M4` dynamic power (laser mode) instead of `M3`.
    pub dynamic_power: bool,
    /// mm/min used for travel-time estimates (G0).
    pub rapid_speed: f64,
    /// Extra lead-in/out for raster rows in mm so the head is at speed.
    pub overscan: f64,
    /// Which controller firmware's G-code to write; absent in older files (GRBL).
    #[serde(default)]
    pub dialect: Dialect,
    /// Rotary attachment: the Y axis turns the workpiece instead of moving the head.
    #[serde(default)]
    pub rotary: Rotary,
    /// Controller acceleration in mm/s² for time estimates (GRBL `$120`/`$121`); 0 = ignore.
    #[serde(default)]
    pub acceleration: f64,
    /// Where a layer sets no overscan of its own, run up for the distance the head needs to reach
    /// the layer's speed at `acceleration` (v²/2a) instead of a fixed length.
    #[serde(default)]
    pub overscan_by_speed: bool,
}

/// A rotary attachment. The controller's Y steps are calibrated for a roller or chuck of
/// `reference_diameter`; a workpiece of another diameter needs Y distances scaled by
/// `reference_diameter / object_diameter` so the surface moves by the distance the design asks for.
/// The job's Y extent is then the object's circumference, `PI * object_diameter`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rotary {
    pub enabled: bool,
    pub reference_diameter: f64,
    pub object_diameter: f64,
}

impl Default for Rotary {
    fn default() -> Self {
        Rotary { enabled: false, reference_diameter: 50.0, object_diameter: 50.0 }
    }
}

impl Rotary {
    /// The factor applied to Y distances, or 1 when the attachment is off or the numbers are unusable.
    pub fn y_scale(&self) -> f64 {
        let ok = |d: f64| d.is_finite() && d > 0.0;
        if self.enabled && ok(self.reference_diameter) && ok(self.object_diameter) { self.reference_diameter / self.object_diameter } else { 1.0 }
    }

    /// Length of the workpiece's surface around its circumference, mm.
    pub fn circumference(&self) -> f64 {
        std::f64::consts::PI * self.object_diameter
    }
}

/// G-code flavours. GRBL is the one tested on a (simulated) controller; the others follow the
/// public firmware documentation (Marlin: marlinfw.org M3; Smoothieware: smoothieware.org/laser).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    #[default]
    Grbl,
    /// `M3 I` inline power with `S` in the firmware's configured unit (PWM 0-255 by default).
    Marlin,
    /// No spindle commands; `S` is a 0-1 fraction on each `G1`.
    Smoothie,
}

impl Dialect {
    pub const ALL: [Dialect; 3] = [Dialect::Grbl, Dialect::Marlin, Dialect::Smoothie];

    pub fn label(self) -> &'static str {
        match self {
            Dialect::Grbl => "GRBL",
            Dialect::Marlin => "Marlin (laser)",
            Dialect::Smoothie => "Smoothieware",
        }
    }
}

impl Default for Machine {
    fn default() -> Self {
        Machine {
            name: "Generic GRBL 400x400".into(),
            bed_width: 400.0,
            bed_height: 400.0,
            origin: Origin::BottomLeft,
            max_s: 1000,
            dynamic_power: true,
            rapid_speed: 6000.0,
            overscan: 2.0,
            dialect: Dialect::Grbl,
            rotary: Rotary::default(),
            acceleration: 0.0,
            overscan_by_speed: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    BottomLeft,
    TopLeft,
}

#[cfg(test)]
mod tests {
    use super::*;
    use laserfoundry_geom::rect_path;

    #[test]
    fn every_anchor_has_its_own_name() {
        let names: std::collections::HashSet<_> = Anchor::ALL.iter().map(|a| a.label()).collect();
        assert_eq!(names.len(), Anchor::ALL.len());
    }

    #[test]
    fn add_shape_assigns_ids_and_falls_back_to_layer_zero() {
        let mut doc = Document::default();
        let a = doc.add_shape(Shape::path("a", 99, rect_path(0.0, 0.0, 1.0, 1.0)));
        let b = doc.add_shape(Shape::path("b", 2, rect_path(0.0, 0.0, 1.0, 1.0)));
        assert_ne!(a, b);
        assert_eq!(doc.shape(a).map(|s| s.layer), Some(0));
        assert_eq!(doc.shape(b).map(|s| s.layer), Some(2));
    }

    #[test]
    fn world_bounds_apply_transform() {
        let mut s = Shape::path("r", 0, rect_path(0.0, 0.0, 10.0, 10.0));
        s.transform = Affine::translate((5.0, 7.0));
        let b = s.world_bounds();
        assert_eq!(b, Some(Rect::new(5.0, 7.0, 15.0, 17.0)));
    }

    #[test]
    fn groups_expand_and_duplicates_form_new_groups() {
        let mut doc = Document::default();
        let ids: Vec<ShapeId> = (0..3).map(|i| doc.add_shape(Shape::path(format!("s{i}"), 0, rect_path(0.0, 0.0, 1.0, 1.0)))).collect();
        for id in &ids[..2] {
            doc.shape_mut(*id).unwrap().group = Some(500);
        }
        assert_eq!(doc.expand_groups(&[ids[0]]), vec![ids[0], ids[1]], "a member brings its group");
        assert_eq!(doc.expand_groups(&[ids[2]]), vec![ids[2]]);
        let copies = doc.duplicate_shapes(&ids[..2], Affine::translate((10.0, 0.0)));
        let (g0, g1) = (doc.shape(copies[0]).unwrap().group, doc.shape(copies[1]).unwrap().group);
        assert!(g0.is_some() && g0 == g1 && g0 != Some(500), "the copies share a fresh group");
        assert_eq!(doc.expand_groups(&[ids[0]]).len(), 2, "the original group is unchanged");
        assert!(doc.duplicate_shapes(&[9999], Affine::IDENTITY).is_empty());
    }

    #[test]
    fn anchors_pick_the_expected_points() {
        let b = Rect::new(10.0, 20.0, 30.0, 60.0);
        assert_eq!(Anchor::TopLeft.point(b), laserfoundry_geom::Point::new(10.0, 20.0));
        assert_eq!(Anchor::Center.point(b), laserfoundry_geom::Point::new(20.0, 40.0));
        assert_eq!(Anchor::BottomRight.point(b), laserfoundry_geom::Point::new(30.0, 60.0));
        assert_eq!(Anchor::ALL.len(), 9);
    }

    #[test]
    fn base64_round_trips_and_rejects_junk() {
        for data in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0u8, 255, 128, 7, 1, 2][..]] {
            assert_eq!(base64::decode(&base64::encode(data)).unwrap(), data);
        }
        assert_eq!(base64::encode(b"Man"), "TWFu");
        assert_eq!(base64::decode("TWF u\n"), Some(b"Man".to_vec()), "whitespace is ignored");
        assert_eq!(base64::decode("TW!u"), None);
    }

    #[test]
    fn applying_a_preset_keeps_the_layers_identity() {
        let mut preset = Layer::numbered(5);
        preset.speed = 321.0;
        preset.max_power = 77.0;
        preset.passes = 3;
        preset.output = false;
        let mut layer = Layer::numbered(2);
        layer.name = "Mine".into();
        layer.apply_settings(&preset);
        assert_eq!((layer.id, layer.name.as_str(), layer.color.as_str(), layer.output), (2, "Mine", Layer::numbered(2).color.as_str(), true));
        assert_eq!((layer.speed, layer.max_power, layer.passes), (321.0, 77.0, 3));
    }

    #[test]
    fn old_files_without_presets_load() {
        let mut v = serde_json::to_value(Document::default()).unwrap();
        v.as_object_mut().unwrap().remove("presets");
        assert!(serde_json::from_value::<Document>(v).unwrap().presets.is_empty());
    }

    #[test]
    fn rotary_scaling_follows_the_diameters_and_ignores_bad_numbers() {
        let r = Rotary { enabled: true, reference_diameter: 60.0, object_diameter: 30.0 };
        assert_eq!(r.y_scale(), 2.0);
        assert!((r.circumference() - 30.0 * std::f64::consts::PI).abs() < 1e-9);
        assert_eq!(Rotary { enabled: false, ..r }.y_scale(), 1.0);
        for bad in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(Rotary { object_diameter: bad, ..r }.y_scale(), 1.0, "{bad}");
        }
    }

    #[test]
    fn old_files_without_a_dialect_are_grbl() {
        let mut v = serde_json::to_value(Document::default()).unwrap();
        v["machine"].as_object_mut().unwrap().remove("dialect");
        let doc: Document = serde_json::from_value(v).unwrap();
        assert_eq!(doc.machine.dialect, Dialect::Grbl);
    }

    #[test]
    fn old_files_without_optimize_settings_load_with_the_defaults() {
        let mut v = serde_json::to_value(Document::default()).unwrap();
        v.as_object_mut().unwrap().remove("optimize");
        let doc: Document = serde_json::from_value(v).unwrap();
        assert_eq!(doc.optimize, Optimize::default());
    }

    #[test]
    fn old_files_without_job_origin_still_load() {
        let mut v = serde_json::to_value(Document::default()).unwrap();
        v.as_object_mut().unwrap().remove("job_origin");
        let doc: Document = serde_json::from_value(v).unwrap();
        assert_eq!(doc.job_origin, JobOrigin::default());
    }

    #[test]
    fn colour_follows_the_grey_through_crop_resample_and_sharpen() {
        // 2x1: red then blue.
        let img = RasterImage::new(2, 1, vec![76, 29]).unwrap().with_rgb(vec![255, 0, 0, 0, 0, 255]).unwrap();
        assert!(RasterImage::new(2, 1, vec![1, 2]).unwrap().with_rgb(vec![0; 5]).is_none());
        let c = img.crop(1, 0, 2, 1).unwrap();
        assert_eq!((c.gray.as_slice(), c.rgb.as_deref()), ([29].as_slice(), Some([0, 0, 255].as_slice())));
        let big = img.resampled(4, 2).unwrap();
        assert_eq!(big.rgb.as_ref().map(Vec::len), Some(4 * 2 * 3));
        let sharp = img.sharpened(1.0);
        assert_eq!(sharp.rgb.as_ref().map(Vec::len), Some(6));
        let json = serde_json::to_string(&img).unwrap();
        assert_eq!(serde_json::from_str::<RasterImage>(&json).unwrap(), img);
        let grey = RasterImage::new(2, 1, vec![76, 29]).unwrap();
        assert!(!serde_json::to_string(&grey).unwrap().contains("rgb"), "grey pictures stay as small as before");
        let old = r#"{"width":2,"height":1,"gray":"TB0="}"#;
        assert_eq!(serde_json::from_str::<RasterImage>(old).unwrap().rgb, None);
    }

    #[test]
    fn resampling_averages_when_shrinking_and_interpolates_when_enlarging() {
        let img = RasterImage::new(4, 2, vec![0, 100, 200, 100, 0, 100, 200, 100]).unwrap();
        let half = img.resampled(2, 1).unwrap();
        assert_eq!(half.gray, vec![50, 150], "each output pixel is the mean of a 2x2 block");
        let same = img.resampled(4, 2).unwrap();
        assert_eq!(same.gray, img.gray, "the same size changes nothing");
        let flat = RasterImage::new(3, 3, vec![77; 9]).unwrap();
        for (w, h) in [(1, 1), (2, 5), (7, 7), (30, 3)] {
            let r = flat.resampled(w, h).unwrap();
            assert!(r.gray.iter().all(|v| *v == 77), "a flat picture stays flat at {w}x{h}");
        }
        let big = img.resampled(8, 4).unwrap();
        assert!(big.gray[0] <= big.gray[1] && big.gray[1] <= big.gray[2], "a gradient stays monotonic: {:?}", big.gray);
        assert!(img.resampled(0, 5).is_none() && img.resampled(5, 0).is_none() && img.resampled(20_000, 20_000).is_none());
        let one = RasterImage::new(1, 1, vec![9]).unwrap();
        assert_eq!(one.resampled(3, 3).unwrap().gray, vec![9; 9]);
    }

    #[test]
    fn cropping_and_sharpening_images() {
        let img = RasterImage::new(4, 3, (0..12).collect()).unwrap();
        let c = img.crop(1, 1, 3, 3).unwrap();
        assert_eq!((c.width, c.height, c.gray.as_slice()), (2, 2, [5, 6, 9, 10].as_slice()));
        assert_eq!(img.crop(0, 0, 4, 3).unwrap(), img, "the full box is the same picture");
        for (x0, y0, x1, y1) in [(2, 0, 2, 3), (3, 0, 1, 3), (0, 0, 5, 3), (0, 0, 4, 4), (0, 2, 4, 2)] {
            assert!(img.crop(x0, y0, x1, y1).is_none(), "{x0} {y0} {x1} {y1}");
        }
        let flat = RasterImage::new(5, 5, vec![100; 25]).unwrap();
        assert_eq!(flat.sharpened(3.0), flat, "a flat picture has no edges to sharpen");
        let mut gray = vec![100u8; 25];
        gray[12] = 150;
        let spike = RasterImage::new(5, 5, gray).unwrap();
        assert!(spike.sharpened(2.0).gray[12] > 150, "a bright dot gets brighter");
        assert_eq!(spike.sharpened(0.0), spike);
        assert_eq!(spike.sharpened(f64::NAN), spike);
        assert_eq!(spike.sharpened(1e9), spike.sharpened(5.0), "the amount is limited");
        assert_eq!(RasterImage::new(1, 1, vec![7]).unwrap().sharpened(2.0).gray, vec![7]);
    }

    #[test]
    fn image_adjustments_behave() {
        let id = ImageAdjust::default();
        assert!(id.is_identity());
        for v in [0.0, 64.0, 128.0, 255.0] {
            assert!((id.apply(v) - v).abs() < 0.01, "identity at {v}");
        }
        assert!((ImageAdjust { negative: true, ..id }.apply(255.0)).abs() < 0.01);
        assert!(ImageAdjust { brightness: 0.2, ..id }.apply(100.0) > 150.0);
        assert!(ImageAdjust { contrast: 0.5, ..id }.apply(200.0) > 200.0 && ImageAdjust { contrast: 0.5, ..id }.apply(50.0) < 50.0);
        assert!(ImageAdjust { gamma: 2.0, ..id }.apply(64.0) > 100.0, "gamma > 1 brightens midtones");
    }

    #[test]
    fn hostile_image_adjustments_stay_in_range() {
        for adjust in [
            ImageAdjust { brightness: f64::NAN, contrast: f64::INFINITY, gamma: f64::NAN, negative: true },
            ImageAdjust { brightness: 5.0, contrast: 5.0, gamma: 0.0, negative: false },
            ImageAdjust { brightness: -9.0, contrast: -9.0, gamma: -3.0, negative: true },
        ] {
            for v in [0.0, 128.0, 255.0, f32::NAN, f32::INFINITY, -50.0, 1e9] {
                let r = adjust.apply(v);
                assert!((0.0..=255.0).contains(&r), "{adjust:?} at {v} gave {r}");
            }
        }
    }

    #[test]
    fn raster_rejects_mismatched_buffers() {
        assert!(RasterImage::new(2, 2, vec![0; 3]).is_none());
        assert!(RasterImage::new(0, 0, vec![]).is_none());
        assert!(RasterImage::new(2, 2, vec![0; 4]).is_some());
    }

    #[test]
    fn document_serde_round_trip_including_images() {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("r", 1, rect_path(0.0, 0.0, 3.0, 4.0)));
        for len in 1..=7u8 {
            let image = Arc::new(RasterImage::new(u32::from(len), 1, (0..len).map(|i| i * 37).collect()).unwrap());
            doc.add_shape(Shape {
                id: 0,
                name: "i".into(),
                layer: 0,
                transform: Affine::IDENTITY,
                kind: ShapeKind::Image { image, width_mm: 1.0, height_mm: 1.0, adjust: ImageAdjust::default() },
                cut: None,
                group: None,
                order: 0,
            });
        }
        let json = serde_json::to_string(&doc).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back, doc);
    }
}
