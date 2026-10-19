use laserfoundry_doc::{Dither, Document, Layer, RasterImage, Shape, ShapeKind};
use laserfoundry_geom::{Point, bounds_of_polylines};

use crate::{MAX_RASTER_CELLS, Move, PlanError};

/// Burn intensity per cell, 0 (skip) ..= 1 (full), for a `w`×`h` grid of 0..=255 gray samples.
pub fn dither(gray: &[f32], w: usize, h: usize, mode: Dither) -> Vec<f32> {
    let mut out = vec![0.0f32; gray.len()];
    match mode {
        Dither::Grayscale => {
            for (o, g) in out.iter_mut().zip(gray) {
                *o = 1.0 - g / 255.0;
            }
        }
        Dither::Threshold => {
            for (o, g) in out.iter_mut().zip(gray) {
                *o = if *g < 128.0 { 1.0 } else { 0.0 };
            }
        }
        Dither::Atkinson => diffuse(gray, w, h, &mut out, &ATKINSON, 8.0),
        Dither::Jarvis => diffuse(gray, w, h, &mut out, &JARVIS, 48.0),
        Dither::Ordered => {
            let matrix = bayer8();
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let (Some(o), Some(g)) = (out.get_mut(i), gray.get(i)) else { continue };
                    let level = (f32::from(matrix[(y % 8) * 8 + x % 8]) + 0.5) / 64.0 * 255.0;
                    *o = if *g < level { 1.0 } else { 0.0 };
                }
            }
        }
        Dither::Halftone => {
            // A 45 degree screen: two crossed cosines make a dot that grows from the cell centre.
            // The spot function is not uniformly distributed, so the darkness is mapped through its
            // quantiles; that makes the share of burned pixels follow the picture's tone.
            let cell = HALFTONE_CELL_PX;
            let k = std::f32::consts::TAU / cell;
            let r = std::f32::consts::FRAC_1_SQRT_2;
            let quantiles = spot_quantiles();
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let (Some(o), Some(g)) = (out.get_mut(i), gray.get(i)) else { continue };
                    let (u, v) = ((x + y) as f32 * r, (x as f32 - y as f32) * r);
                    let level = spot((u * k).cos(), (v * k).cos());
                    let darkness = (1.0 - *g / 255.0).clamp(0.0, 1.0);
                    let cut = quantiles[(darkness * (QUANTILES - 1) as f32).round() as usize];
                    *o = if darkness >= 1.0 || level < cut { 1.0 } else { 0.0 };
                }
            }
        }
        Dither::FloydSteinberg => {
            let mut buf = gray.to_vec();
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let old = buf.get(i).copied().unwrap_or(255.0);
                    let new = if old < 128.0 { 0.0 } else { 255.0 };
                    if let Some(o) = out.get_mut(i) {
                        *o = if new == 0.0 { 1.0 } else { 0.0 };
                    }
                    let err = old - new;
                    let mut spread = |dx: isize, dy: usize, k: f32| {
                        let nx = x as isize + dx;
                        if nx < 0 || nx as usize >= w || y + dy >= h {
                            return;
                        }
                        if let Some(c) = buf.get_mut((y + dy) * w + nx as usize) {
                            *c += err * k;
                        }
                    };
                    spread(1, 0, 7.0 / 16.0);
                    spread(-1, 1, 3.0 / 16.0);
                    spread(0, 1, 5.0 / 16.0);
                    spread(1, 1, 1.0 / 16.0);
                }
            }
        }
    }
    out
}

/// A world-space sampling grid: `nx`×`ny` cells of `pitch` mm with its top-left corner at `(x0, y0)`.
#[derive(Debug, Clone, Copy)]
pub struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub nx: usize,
    pub ny: usize,
    pub pitch: f64,
}

/// Samples an image shape on `grid` (bilinear). Cells outside the image are white (255); a
/// non-image shape yields an all-white grid.
pub fn sample_grid(shape: &Shape, grid: &Grid) -> Vec<f32> {
    let mut out = vec![255.0f32; grid.nx.saturating_mul(grid.ny)];
    let ShapeKind::Image { image: img, width_mm: w_mm, height_mm: h_mm, adjust } = &shape.kind else { return out };
    let inv = shape.transform.inverse();
    for iy in 0..grid.ny {
        for ix in 0..grid.nx {
            let world = Point::new(grid.x0 + (ix as f64 + 0.5) * grid.pitch, grid.y0 + (iy as f64 + 0.5) * grid.pitch);
            let local = inv * world;
            if local.x < 0.0 || local.y < 0.0 || local.x >= *w_mm || local.y >= *h_mm {
                continue;
            }
            let fx = local.x / w_mm * f64::from(img.width) - 0.5;
            let fy = local.y / h_mm * f64::from(img.height) - 0.5;
            if let Some(c) = out.get_mut(iy * grid.nx + ix) {
                // Only pixels inside the image are adjusted: the white outside must stay white, or a
                // negative would burn everything around the picture.
                let v = bilinear(img, fx, fy);
                *c = if adjust.is_identity() { v } else { adjust.apply(v) };
            }
        }
    }
    out
}

fn bilinear(img: &RasterImage, fx: f64, fy: f64) -> f32 {
    let px = |x: i64, y: i64| -> f32 {
        let x = x.clamp(0, img.width as i64 - 1) as usize;
        let y = y.clamp(0, img.height as i64 - 1) as usize;
        img.gray.get(y * img.width as usize + x).copied().unwrap_or(255) as f32
    };
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let top = px(x0, y0) * (1.0 - tx) + px(x0 + 1, y0) * tx;
    let bottom = px(x0, y0 + 1) * (1.0 - tx) + px(x0 + 1, y0 + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

pub(crate) fn plan_image(doc: &Document, layer: &Layer, shape: &Shape) -> Result<Vec<Move>, PlanError> {
    if !matches!(shape.kind, ShapeKind::Image { .. }) {
        return Ok(Vec::new());
    }
    let Some(b) = bounds_of_polylines(&shape.world_polylines()) else { return Ok(Vec::new()) };
    let pitch = layer.interval;
    let nx = ((b.x1 - b.x0) / pitch).ceil();
    let ny = ((b.y1 - b.y0) / pitch).ceil();
    if !(nx.is_finite() && ny.is_finite()) || nx < 1.0 || ny < 1.0 {
        return Ok(Vec::new());
    }
    let cells = (nx as usize).saturating_mul(ny as usize);
    if cells > MAX_RASTER_CELLS {
        return Err(PlanError::RasterTooLarge(cells));
    }
    let (nx, ny) = (nx as usize, ny as usize);
    let gray = sample_grid(shape, &Grid { x0: b.x0, y0: b.y0, nx, ny, pitch });
    let intensity = dither(&gray, nx, ny, layer.dither);

    let over = crate::overscan_for(doc, layer, doc.machine.overscan);
    let (min_p, max_p) = (layer.min_power.clamp(0.0, 100.0), layer.max_power.clamp(0.0, 100.0));
    let x_at = |i: usize| b.x0 + i as f64 * pitch;
    // With absolute placement the bed edges are known: a picture flush with an edge gets a shorter
    // lead-in rather than a job that cannot run. (Relative placements cannot know the room.)
    let absolute = doc.job_origin.start_from == laserfoundry_doc::StartFrom::Absolute;
    let room = |x: f64| if absolute { x.clamp(0.0, doc.machine.bed_width) } else { x };
    let mut moves = Vec::new();
    for (iy, row) in intensity.chunks_exact(nx).enumerate() {
        let Some(first) = row.iter().position(|v| *v > 0.0) else { continue };
        let last = row.iter().rposition(|v| *v > 0.0).unwrap_or(first);
        let (from, to) = (first, last + 1);
        let y = b.y0 + (iy as f64 + 0.5) * pitch;
        let reverse = layer.bidirectional && iy % 2 == 1;
        let (start_x, end_x, dir) = if reverse { (x_at(to), x_at(from), -1.0) } else { (x_at(from), x_at(to), 1.0) };

        moves.push(Move::Travel(Point::new(room(start_x - dir * over), y)));
        if over > 0.0 {
            moves.push(Move::Cut { to: Point::new(start_x, y), power: 0.0 });
        }
        let order: Vec<usize> = if reverse { (from..to).rev().collect() } else { (from..to).collect() };
        let mut run_power: Option<f64> = None;
        for c in order {
            let v = row.get(c).copied().unwrap_or(0.0) as f64;
            // Quarter-percent steps keep equal-looking runs merged.
            let power = if v > 0.0 { ((min_p + (max_p - min_p) * v) * 4.0).round() / 4.0 } else { 0.0 };
            let end = if reverse { x_at(c) } else { x_at(c + 1) };
            match moves.last_mut() {
                Some(Move::Cut { to, power: p }) if run_power == Some(power) && *p == power => to.x = end,
                _ => moves.push(Move::Cut { to: Point::new(end, y), power }),
            }
            run_power = Some(power);
        }
        if over > 0.0 {
            moves.push(Move::Cut { to: Point::new(room(end_x + dir * over), y), power: 0.0 });
        }
    }
    Ok(moves)
}

/// Steps in the halftone quantile table.
const QUANTILES: usize = 256;

fn spot(cos_a: f32, cos_b: f32) -> f32 {
    0.5 + (cos_a + cos_b) * 0.25
}

/// Values of the spot function at which a given share (index / 255) of a cell lies below.
fn spot_quantiles() -> [f32; QUANTILES] {
    const N: usize = 64;
    let mut values: Vec<f32> = Vec::with_capacity(N * N);
    for i in 0..N {
        for j in 0..N {
            let a = (i as f32 + 0.5) / N as f32 * std::f32::consts::TAU;
            let b = (j as f32 + 0.5) / N as f32 * std::f32::consts::TAU;
            values.push(spot(a.cos(), b.cos()));
        }
    }
    values.sort_by(f32::total_cmp);
    let mut table = [0.0f32; QUANTILES];
    for (q, slot) in table.iter_mut().enumerate() {
        let idx = (q * (values.len() - 1)) / (QUANTILES - 1);
        *slot = values.get(idx).copied().unwrap_or(1.0);
    }
    table
}

/// Halftone cell size in image pixels.
const HALFTONE_CELL_PX: f32 = 6.0;

/// Error-diffusion kernels as `(dx, dy, weight)`, with the divisor passed separately.
const ATKINSON: [(isize, usize, f32); 6] = [(1, 0, 1.0), (2, 0, 1.0), (-1, 1, 1.0), (0, 1, 1.0), (1, 1, 1.0), (0, 2, 1.0)];
const JARVIS: [(isize, usize, f32); 12] = [
    (1, 0, 7.0),
    (2, 0, 5.0),
    (-2, 1, 3.0),
    (-1, 1, 5.0),
    (0, 1, 7.0),
    (1, 1, 5.0),
    (2, 1, 3.0),
    (-2, 2, 1.0),
    (-1, 2, 3.0),
    (0, 2, 5.0),
    (1, 2, 3.0),
    (2, 2, 1.0),
];

/// Thresholds each pixel and pushes the error onto its neighbours with the given kernel.
fn diffuse(gray: &[f32], w: usize, h: usize, out: &mut [f32], kernel: &[(isize, usize, f32)], divisor: f32) {
    let mut buf = gray.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let old = buf.get(i).copied().unwrap_or(255.0);
            let new = if old < 128.0 { 0.0 } else { 255.0 };
            if let Some(o) = out.get_mut(i) {
                *o = if new == 0.0 { 1.0 } else { 0.0 };
            }
            let err = old - new;
            for &(dx, dy, weight) in kernel {
                let nx = x as isize + dx;
                if nx < 0 || nx as usize >= w || y + dy >= h {
                    continue;
                }
                if let Some(c) = buf.get_mut((y + dy) * w + nx as usize) {
                    *c += err * weight / divisor;
                }
            }
        }
    }
}

/// The 8x8 Bayer index matrix (0..=63), built by the usual recursive doubling.
fn bayer8() -> [u8; 64] {
    let mut m = [0u8; 64];
    for y in 0..8usize {
        for x in 0..8usize {
            // Interleave the bits of (x ^ y) and y, reversed: the classic closed form.
            let (mut v, mut xor, mut yy) = (0u8, x ^ y, y);
            for _ in 0..3 {
                v = (v << 2) | (((yy & 1) as u8) << 1) | ((xor & 1) as u8);
                yy >>= 1;
                xor >>= 1;
            }
            m[y * 8 + x] = v;
        }
    }
    m
}
