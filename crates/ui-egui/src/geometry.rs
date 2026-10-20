//! Per-revision caches of derived data: flattened shape outlines for drawing and hit-testing,
//! and the planned toolpath for the preview.

use laserfoundry_doc::{LayerId, ShapeId};
use laserfoundry_engine::Session;
use laserfoundry_geom::{Point, Polyline, Rect, bounds_of_polylines};
use laserfoundry_toolpath::{Motion, Move, Timeline, move_times, plan};

/// Preview segments drawn at most; huge raster jobs are truncated for display only.
pub const MAX_PREVIEW_SEGMENTS: usize = 400_000;

pub struct Item {
    pub id: ShapeId,
    pub layer: LayerId,
    pub lines: Vec<Polyline>,
    pub bounds: Rect,
    /// Filled-in for picking: images and text are picked anywhere inside their bounds.
    pub is_image: bool,
    pub is_text: bool,
}

#[derive(Default)]
pub struct GeometryCache {
    revision: Option<u64>,
    pub items: Vec<Item>,
}

impl GeometryCache {
    pub fn refresh(&mut self, s: &Session) {
        if self.revision == Some(s.revision) {
            return;
        }
        self.revision = Some(s.revision);
        self.items = s
            .doc
            .shapes
            .iter()
            .filter_map(|sh| {
                let lines = sh.world_polylines();
                let bounds = bounds_of_polylines(&lines)?;
                Some(Item {
                    id: sh.id,
                    layer: sh.layer,
                    lines,
                    bounds,
                    is_image: matches!(sh.kind, laserfoundry_doc::ShapeKind::Image { .. }),
                    is_text: matches!(sh.kind, laserfoundry_doc::ShapeKind::Text { .. }),
                })
            })
            .collect();
    }

    /// Topmost shape whose outline is within `tol` mm of `p`, or whose closed outline contains it.
    pub fn pick(&self, p: Point, tol: f64) -> Option<ShapeId> {
        self.items
            .iter()
            .rev()
            .find(|it| {
                // Thin strokes are hard to hit, so text counts as a block.
                (it.is_text && it.bounds.inflate(tol, tol).contains(p)) || it.lines.iter().any(|l| l.contains(p) || l.distance_to(p) <= tol)
            })
            .map(|it| it.id)
    }
}

pub struct PreviewSegment {
    pub a: Point,
    pub b: Point,
    pub layer: LayerId,
    /// 0 for travel and zero-power moves.
    pub power: f32,
    /// Feed rate the move runs at, mm/min.
    pub speed: f32,
    /// Index of the planned operation, and when in a pass of it the move starts and ends (seconds).
    pub op: u32,
    pub t0: f32,
    pub t1: f32,
}

#[derive(Default)]
pub struct PreviewCache {
    /// What the preview was built from: the document revision, and the selection when only that is cut.
    key: Option<(u64, Option<Vec<ShapeId>>)>,
    pub segments: Vec<PreviewSegment>,
    pub error: Option<String>,
    pub truncated: bool,
    pub timeline: Timeline,
    /// Lowest and highest power and feed among the burning moves (for colouring).
    pub power_range: (f32, f32),
    pub speed_range: (f32, f32),
}

impl PreviewCache {
    pub fn refresh(&mut self, s: &Session) {
        let key = (s.revision, s.job_selection_only.then(|| s.selection.clone()));
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        self.segments.clear();
        self.error = None;
        self.truncated = false;
        self.timeline = Timeline::default();
        self.power_range = (0.0, 1.0);
        self.speed_range = (0.0, 1.0);
        let job_doc = s.job_doc();
        match plan(&job_doc) {
            Err(e) => self.error = Some(e.to_string()),
            Ok(job) => {
                let rapid = job_doc.machine.rapid_speed;
                let motion = Motion { acceleration: job_doc.machine.acceleration, junction_deviation: job_doc.machine.junction_deviation };
                self.timeline = Timeline::with_motion(&job, rapid, motion);
                // The head's real start position is unknown, so the first move draws nothing.
                let mut at: Option<Point> = None;
                // Time follows the same path as the timeline model: from the origin, across operations.
                let mut timed = Point::ORIGIN;
                'ops: for (index, op) in job.ops.iter().enumerate() {
                    let mut local = 0.0_f64;
                    let times = move_times(op, timed, rapid, motion);
                    for (m, &dt) in op.moves.iter().zip(&times) {
                        if self.segments.len() >= MAX_PREVIEW_SEGMENTS {
                            self.truncated = true;
                            break 'ops;
                        }
                        let power = match m {
                            Move::Travel(_) => 0.0,
                            Move::Cut { power, .. } => *power as f32,
                        };
                        if let Some(a) = at {
                            let (t0, t1) = (local as f32, (local + dt) as f32);
                            self.segments.push(PreviewSegment {
                                a,
                                b: m.target(),
                                layer: op.layer,
                                power,
                                speed: op.speed as f32,
                                op: index as u32,
                                t0,
                                t1,
                            });
                        }
                        local += dt;
                        timed = m.target();
                        at = Some(m.target());
                    }
                }
            }
        }
        let mut power = (f32::INFINITY, f32::NEG_INFINITY);
        let mut speed = (f32::INFINITY, f32::NEG_INFINITY);
        for seg in self.segments.iter().filter(|s| s.power > 0.0) {
            power = (power.0.min(seg.power), power.1.max(seg.power));
            speed = (speed.0.min(seg.speed), speed.1.max(seg.speed));
        }
        if power.0.is_finite() {
            self.power_range = power;
            self.speed_range = speed;
        }
    }
}

/// Thumbnails of image shapes (tone adjustments applied), uploaded as textures and reused until
/// the picture or its adjustment changes.
#[derive(Default)]
pub struct ImageTextures {
    map: std::collections::HashMap<ShapeId, (ThumbKey, egui::TextureHandle)>,
}

#[derive(PartialEq, Clone, Copy)]
struct ThumbKey {
    /// Address of the shared pixel buffer: changes when the picture is replaced.
    buffer: usize,
    adjust: [u64; 3],
    negative: bool,
}

/// Largest thumbnail side, in pixels.
const THUMB_MAX: usize = 256;

impl ImageTextures {
    /// The texture for an image shape, (re)built when needed. `None` for non-images.
    pub fn get(&mut self, ctx: &egui::Context, shape: &laserfoundry_doc::Shape) -> Option<egui::TextureId> {
        let laserfoundry_doc::ShapeKind::Image { image, adjust, .. } = &shape.kind else { return None };
        let key = ThumbKey {
            buffer: std::sync::Arc::as_ptr(image) as usize,
            adjust: [adjust.brightness.to_bits(), adjust.contrast.to_bits(), adjust.gamma.to_bits()],
            negative: adjust.negative,
        };
        if let Some((k, handle)) = self.map.get(&shape.id)
            && *k == key
        {
            return Some(handle.id());
        }
        let (w, h) = thumb_size(image.width as usize, image.height as usize);
        let pixels = downscale(image, w, h, adjust);
        let handle = ctx.load_texture(format!("image-{}", shape.id), egui::ColorImage::from_gray([w, h], &pixels), egui::TextureOptions::LINEAR);
        let id = handle.id();
        self.map.insert(shape.id, (key, handle));
        Some(id)
    }

    #[cfg(test)]
    pub fn map_len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub fn cached_key(&self, id: ShapeId) -> Option<([u64; 3], bool, usize)> {
        self.map.get(&id).map(|(k, _)| (k.adjust, k.negative, k.buffer))
    }

    /// Forgets textures of shapes that no longer exist.
    pub fn retain(&mut self, alive: impl Fn(ShapeId) -> bool) {
        self.map.retain(|id, _| alive(*id));
    }
}

fn thumb_size(w: usize, h: usize) -> (usize, usize) {
    let scale = (THUMB_MAX as f64 / w.max(h).max(1) as f64).min(1.0);
    (((w as f64 * scale).round() as usize).max(1), ((h as f64 * scale).round() as usize).max(1))
}

/// Box-average downscale with the tone adjustment applied.
fn downscale(img: &laserfoundry_doc::RasterImage, tw: usize, th: usize, adjust: &laserfoundry_doc::ImageAdjust) -> Vec<u8> {
    let (sw, sh) = (img.width as usize, img.height as usize);
    let mut out = Vec::with_capacity(tw * th);
    for ty in 0..th {
        let (y0, y1) = (ty * sh / th, ((ty + 1) * sh / th).max(ty * sh / th + 1).min(sh));
        for tx in 0..tw {
            let (x0, x1) = (tx * sw / tw, ((tx + 1) * sw / tw).max(tx * sw / tw + 1).min(sw));
            let (mut sum, mut n) = (0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += u32::from(img.gray.get(y * sw + x).copied().unwrap_or(255));
                    n += 1;
                }
            }
            let avg = if n == 0 { 255.0 } else { sum as f32 / n as f32 };
            out.push(adjust.apply(avg).round() as u8);
        }
    }
    out
}
