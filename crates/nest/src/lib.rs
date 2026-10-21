//! Nesting: packing parts into a region of the bed so they use as little of it as they can.
//!
//! This is the pixel-based (raster) approach from the nesting literature: the region becomes a grid, each part becomes a mask of
//! the grid cells it touches (every cell its outline passes through, plus every cell inside a closed outline), and parts are
//! placed one at a time, largest first, at the first free spot found scanning from the top-left, trying each allowed rotation.
//! The masks are conservative: a cell counts as taken if any piece of the part touches it, so parts never overlap, and the gap
//! between parts is made by thickening each mask before it is tested, so the real distance is at least the gap asked for. The
//! price is resolution: a part can waste up to about one cell of room on each side. Holes in a part are not used by other parts.
//!
//! Nothing here panics on odd input: numbers are checked, sizes and the amount of work are capped, and parts that cannot be
//! placed are reported, not forced in.

use laserfoundry_geom::{Affine, Point, Polyline, Rect, Vec2};

#[cfg(test)]
mod tests;

/// Most parts one nesting may place.
pub const MAX_PARTS: usize = 500;
/// Most points, over all parts, one nesting will look at.
pub const MAX_POINTS: usize = 5_000_000;
/// Most rotations tried for each part.
pub const MAX_ROTATIONS: u32 = 72;

/// Most cells in the grid.
const MAX_CELLS: f64 = 1_000_000.0;
/// Cells across the longer side of the region when the resolution is chosen automatically.
const AUTO_CELLS_ACROSS: f64 = 240.0;
/// Smallest cell (mm) the program will use.
const MIN_CELL: f64 = 0.02;
/// Most cells a gap may take; a bigger gap makes the cells bigger instead.
const MAX_GAP_CELLS: f64 = 64.0;
/// Most grid cells inspected in all before the search stops (the parts left over are reported as not placed).
const MAX_WORK: u64 = 200_000_000;

/// One thing to place: its outlines in workspace mm. Closed outlines are solid; open ones (lines) take only their own path.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Part {
    pub outlines: Vec<Polyline>,
}

/// How to nest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Least distance between two parts, mm.
    pub gap: f64,
    /// Room kept free all round the region's edge, mm.
    pub margin: f64,
    /// How many evenly spaced turns of a full circle are tried for each part (1 = parts keep their angle).
    pub rotations: u32,
    /// Cell size in mm; `None` picks one from the size of the region. Smaller packs tighter and takes longer.
    pub resolution: Option<f64>,
}

impl Default for Options {
    fn default() -> Self {
        Self { gap: 2.0, margin: 0.0, rotations: 4, resolution: None }
    }
}

/// Where one part goes: turn it about `center` by `rotation` radians, then move it by `shift`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Index of the part in the list given.
    pub part: usize,
    pub rotation: f64,
    pub center: Point,
    pub shift: Vec2,
}

impl Placement {
    /// The change to apply to everything in the part.
    pub fn affine(&self) -> Affine {
        Affine::translate(self.shift) * Affine::rotate_about(self.rotation, self.center)
    }
}

/// The result of [`nest`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Nesting {
    /// The parts that fit, in the order given.
    pub placed: Vec<Placement>,
    /// Indices of the parts that did not fit (or had nothing to place), in the order given.
    pub unplaced: Vec<usize>,
    /// The cell size used, mm.
    pub cell: f64,
    /// True when the search hit its work limit; the parts it had not reached are in `unplaced`.
    pub stopped_early: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum NestError {
    #[error("the region to nest in is empty or not a usable size")]
    Region,
    #[error("{0} is not a usable setting")]
    Option(&'static str),
    #[error("at most {MAX_PARTS} parts can be nested at once")]
    TooManyParts,
    #[error("part {0} has a coordinate that is not a finite number")]
    NotFinite(usize),
    #[error("the parts have too many points to nest")]
    TooComplex,
}

/// Packs `parts` into `region` (workspace mm).
pub fn nest(parts: &[Part], region: Rect, options: &Options) -> Result<Nesting, NestError> {
    check_options(options)?;
    if ![region.x0, region.y0, region.x1, region.y1].iter().all(|v| v.is_finite()) {
        return Err(NestError::Region);
    }
    let inner = Rect::new(region.x0 + options.margin, region.y0 + options.margin, region.x1 - options.margin, region.y1 - options.margin);
    let (width, height) = (inner.width(), inner.height());
    if !(width > 0.0 && height > 0.0) {
        return Err(NestError::Region);
    }
    check_parts(parts)?;

    let cell = choose_cell(width, height, options);
    let gap_cells = (options.gap / cell).ceil() as usize;
    let mut grid = Grid::new((width / cell).floor() as usize, (height / cell).floor() as usize, gap_cells);
    let mut out = Nesting { cell, ..Nesting::default() };

    // Biggest first: the big parts are the hard ones to fit, and small ones fill in around them.
    let area = |i: usize| parts.get(i).and_then(bounds).map_or(0.0, |b| b.width() * b.height());
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|&a, &b| area(b).total_cmp(&area(a)).then(a.cmp(&b)));

    let origin = Vec2::new(inner.x0, inner.y0);
    let turns = options.rotations.max(1);
    let mut work = 0_u64;
    let mut slots: Vec<Option<Placement>> = vec![None; parts.len()];
    'parts: for &index in &order {
        let Some(part) = parts.get(index) else { continue };
        let Some(b) = bounds(part) else { continue };
        let center = b.center();
        let mut best: Option<Fit> = None;
        for turn in 0..turns {
            let angle = std::f64::consts::TAU * f64::from(turn) / f64::from(turns);
            let Some(mask) = Mask::of(part, center, angle, cell, grid.cols, grid.rows) else { continue };
            let probe = mask.probe(gap_cells);
            let limit = best.as_ref().map_or(usize::MAX, |f| f.bottom);
            let found = grid.first_fit(&mask, &probe, limit, &mut work);
            if work > MAX_WORK {
                out.stopped_early = true;
                break 'parts;
            }
            if let Some((left, top)) = found {
                let fit = Fit { bottom: top + mask.rows, left, turn, top, angle, mask };
                if best.as_ref().is_none_or(|f| fit.key() < f.key()) {
                    best = Some(fit);
                }
            }
        }
        if let Some(fit) = best {
            grid.occupy(&fit.mask, fit.left, fit.top);
            let corner = Vec2::new(fit.left as f64 * cell, fit.top as f64 * cell);
            let shift = origin + corner - fit.mask.min.to_vec2();
            if let Some(slot) = slots.get_mut(index) {
                *slot = Some(Placement { part: index, rotation: fit.angle, center, shift });
            }
        }
    }
    for (index, slot) in slots.into_iter().enumerate() {
        match slot {
            Some(p) => out.placed.push(p),
            None => out.unplaced.push(index),
        }
    }
    Ok(out)
}

fn check_options(o: &Options) -> Result<(), NestError> {
    if !(o.gap.is_finite() && (0.0..=1000.0).contains(&o.gap)) {
        return Err(NestError::Option("the gap"));
    }
    if !(o.margin.is_finite() && (0.0..=100_000.0).contains(&o.margin)) {
        return Err(NestError::Option("the margin"));
    }
    if !(1..=MAX_ROTATIONS).contains(&o.rotations) {
        return Err(NestError::Option("the number of rotations"));
    }
    if o.resolution.is_some_and(|r| !(r.is_finite() && r > 0.0)) {
        return Err(NestError::Option("the resolution"));
    }
    Ok(())
}

fn check_parts(parts: &[Part]) -> Result<(), NestError> {
    if parts.len() > MAX_PARTS {
        return Err(NestError::TooManyParts);
    }
    let mut points = 0_usize;
    for (index, part) in parts.iter().enumerate() {
        for line in &part.outlines {
            points = points.saturating_add(line.points.len());
            if line.points.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
                return Err(NestError::NotFinite(index));
            }
        }
    }
    if points > MAX_POINTS { Err(NestError::TooComplex) } else { Ok(()) }
}

/// The cell size in mm: the one asked for, or one that gives about [`AUTO_CELLS_ACROSS`] across, made bigger when the grid or
/// the gap would otherwise have too many cells.
fn choose_cell(width: f64, height: f64, options: &Options) -> f64 {
    let wanted = options.resolution.unwrap_or(width.max(height) / AUTO_CELLS_ACROSS).max(MIN_CELL);
    wanted.max((width * height / MAX_CELLS).sqrt()).max(options.gap / MAX_GAP_CELLS)
}

fn bounds(part: &Part) -> Option<Rect> {
    let mut points = part.outlines.iter().flat_map(|l| l.points.iter());
    let first = points.next()?;
    let mut r = Rect::new(first.x, first.y, first.x, first.y);
    for p in points {
        r = Rect::new(r.x0.min(p.x), r.y0.min(p.y), r.x1.max(p.x), r.y1.max(p.y));
    }
    Some(r)
}

/// A spot found for a part at one rotation. The best is the one whose bottom edge is highest, then the leftmost, then the
/// smallest turn.
struct Fit {
    bottom: usize,
    left: usize,
    turn: u32,
    top: usize,
    angle: f64,
    mask: Mask,
}

impl Fit {
    fn key(&self) -> (usize, usize, u32) {
        (self.bottom, self.left, self.turn)
    }
}

/// The cells a part touches, with its bounding box's top-left corner at cell (0, 0).
struct Mask {
    cols: usize,
    rows: usize,
    cells: Vec<bool>,
    /// Where that corner is, in workspace mm, once the part has been turned.
    min: Point,
}

impl Mask {
    /// The mask of `part` turned by `angle` about `center`; `None` when it is too big for a grid of `grid_cols` by `grid_rows`.
    fn of(part: &Part, center: Point, angle: f64, cell: f64, grid_cols: usize, grid_rows: usize) -> Option<Mask> {
        let (sin, cos) = angle.sin_cos();
        let turn = |p: &Point| {
            let (dx, dy) = (p.x - center.x, p.y - center.y);
            Point::new(center.x + dx * cos - dy * sin, center.y + dx * sin + dy * cos)
        };
        let turned: Vec<Vec<Point>> = part.outlines.iter().map(|l| l.points.iter().map(turn).collect()).collect();
        let mut points = turned.iter().flatten();
        let first = points.next()?;
        let (mut min, mut max) = (*first, *first);
        for p in points {
            min = Point::new(min.x.min(p.x), min.y.min(p.y));
            max = Point::new(max.x.max(p.x), max.y.max(p.y));
        }
        let (across, down) = ((max.x - min.x) / cell, (max.y - min.y) / cell);
        if !(across < grid_cols as f64 && down < grid_rows as f64) {
            return None;
        }
        let (cols, rows) = (across.floor() as usize + 1, down.floor() as usize + 1);
        let mut mask = Mask { cols, rows, cells: vec![false; cols * rows], min };
        let mut scan = Scan::new(rows);
        for (line, points) in part.outlines.iter().zip(&turned) {
            let cells: Vec<(f64, f64)> = points.iter().map(|p| ((p.x - min.x) / cell, (p.y - min.y) / cell)).collect();
            mask.draw(&cells, line.closed, &mut scan);
        }
        Some(mask)
    }

    fn mark(&mut self, x: i64, y: i64) {
        if x >= 0
            && y >= 0
            && (x as usize) < self.cols
            && (y as usize) < self.rows
            && let Some(c) = self.cells.get_mut(y as usize * self.cols + x as usize)
        {
            *c = true;
        }
    }

    fn get(&self, x: usize, y: usize) -> bool {
        x < self.cols && y < self.rows && self.cells.get(y * self.cols + x).copied().unwrap_or(false)
    }

    /// Marks the cells one outline touches, and for a closed one the cells inside it.
    fn draw(&mut self, cells: &[(f64, f64)], closed: bool, scan: &mut Scan) {
        match cells {
            [] => {}
            [only] => self.mark(only.0.floor() as i64, only.1.floor() as i64),
            _ => {
                for pair in cells.windows(2) {
                    if let [a, b] = pair {
                        self.segment(*a, *b);
                    }
                }
                if let (true, Some(first), Some(last)) = (closed && cells.len() >= 3, cells.first(), cells.last()) {
                    self.segment(*last, *first);
                    self.fill(cells, scan);
                }
            }
        }
    }

    /// Marks every cell the segment from `a` to `b` passes through, or touches at a corner (a supercover walk across the grid).
    fn segment(&mut self, a: (f64, f64), b: (f64, f64)) {
        let (mut cx, mut cy) = (a.0.floor() as i64, a.1.floor() as i64);
        let (ex, ey) = (b.0.floor() as i64, b.1.floor() as i64);
        self.mark(cx, cy);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (step_x, step_y) = (if dx > 0.0 { 1 } else { -1 }, if dy > 0.0 { 1 } else { -1 });
        let delta = |d: f64| if d != 0.0 { 1.0 / d.abs() } else { f64::INFINITY };
        let (delta_x, delta_y) = (delta(dx), delta(dy));
        let first = |d: f64, from: f64, cell: i64| {
            if d > 0.0 {
                ((cell + 1) as f64 - from) / d
            } else if d < 0.0 {
                (from - cell as f64) / -d
            } else {
                f64::INFINITY
            }
        };
        let (mut next_x, mut next_y) = (first(dx, a.0, cx), first(dy, a.1, cy));
        let mut guard = (ex - cx).abs() + (ey - cy).abs() + 2;
        while (cx, cy) != (ex, ey) && guard > 0 {
            guard -= 1;
            if (next_x - next_y).abs() <= 1e-12 {
                // Straight through a corner of four cells: both cells beside the path count too.
                self.mark(cx + step_x, cy);
                self.mark(cx, cy + step_y);
                cx += step_x;
                cy += step_y;
                next_x += delta_x;
                next_y += delta_y;
            } else if next_x < next_y {
                cx += step_x;
                next_x += delta_x;
            } else {
                cy += step_y;
                next_y += delta_y;
            }
            self.mark(cx, cy);
        }
        self.mark(ex, ey);
    }

    /// Marks the cells whose centres are inside a closed outline (even-odd), using one list of crossings per row.
    fn fill(&mut self, cells: &[(f64, f64)], scan: &mut Scan) {
        let edges = cells.iter().zip(cells.iter().cycle().skip(1));
        for (a, b) in edges.take(cells.len()) {
            if a.1 == b.1 {
                continue;
            }
            let (low, high) = (a.1.min(b.1), a.1.max(b.1));
            let from = (low - 0.5).ceil().max(0.0) as usize;
            let to = ((high - 0.5).ceil().max(0.0) as usize).min(self.rows);
            for row in from..to {
                let y = row as f64 + 0.5;
                let x = a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0);
                scan.add(row, x);
            }
        }
        for (row, crossings) in scan.take() {
            for &[a, b] in crossings.as_chunks::<2>().0 {
                let start = (a - 0.5).ceil().max(0.0) as i64;
                let end = ((b - 0.5).floor() as i64).min(self.cols as i64 - 1);
                for x in start..=end {
                    self.mark(x, row as i64);
                }
            }
        }
    }

    /// The cells this mask must not share with a part already placed: its own cells and every cell within `gap` cells of them.
    /// Positions are in a frame `gap` cells larger on every side, so the mask's own cell (0, 0) is at (`gap`, `gap`).
    fn probe(&self, gap: usize) -> Vec<(usize, usize)> {
        if gap == 0 {
            return (0..self.rows).flat_map(|y| (0..self.cols).map(move |x| (x, y))).filter(|&(x, y)| self.get(x, y)).collect();
        }
        let (width, height) = (self.cols + 2 * gap, self.rows + 2 * gap);
        // Thicken along each row, then along each column, with running totals so the cost does not grow with the gap.
        let mut wide = vec![false; width * self.rows];
        for y in 0..self.rows {
            let sums = prefix((0..self.cols).map(|x| self.get(x, y)));
            for x in 0..width {
                // Own cells from x - 2*gap to x (the frame is shifted by gap on each side).
                let (lo, hi) = (x.saturating_sub(2 * gap), (x + 1).min(self.cols));
                if hi > lo
                    && sums.get(hi).copied().unwrap_or(0) > sums.get(lo).copied().unwrap_or(0)
                    && let Some(c) = wide.get_mut(y * width + x)
                {
                    *c = true;
                }
            }
        }
        let mut out = Vec::new();
        for x in 0..width {
            let sums = prefix((0..self.rows).map(|y| wide.get(y * width + x).copied().unwrap_or(false)));
            for y in 0..height {
                let (lo, hi) = (y.saturating_sub(2 * gap), (y + 1).min(self.rows));
                if hi > lo && sums.get(hi).copied().unwrap_or(0) > sums.get(lo).copied().unwrap_or(0) {
                    out.push((x, y));
                }
            }
        }
        out
    }
}

/// Running totals: element `k` is how many of the first `k` flags are set.
fn prefix(flags: impl Iterator<Item = bool>) -> Vec<u32> {
    let mut sums = vec![0];
    for f in flags {
        let last = sums.last().copied().unwrap_or(0);
        sums.push(last + u32::from(f));
    }
    sums
}

/// Where closed outlines cross the middle of each row of cells, collected while one outline is filled.
struct Scan {
    rows: Vec<Vec<f64>>,
    touched: Vec<usize>,
}

impl Scan {
    fn new(rows: usize) -> Scan {
        Scan { rows: vec![Vec::new(); rows], touched: Vec::new() }
    }

    fn add(&mut self, row: usize, x: f64) {
        if let Some(list) = self.rows.get_mut(row) {
            if list.is_empty() {
                self.touched.push(row);
            }
            list.push(x);
        }
    }

    /// The crossings found, sorted along each row, and clears the lists for the next outline.
    fn take(&mut self) -> Vec<(usize, Vec<f64>)> {
        let rows = std::mem::take(&mut self.touched);
        rows.into_iter()
            .filter_map(|row| {
                let mut list = std::mem::take(self.rows.get_mut(row)?);
                list.sort_by(f64::total_cmp);
                Some((row, list))
            })
            .collect()
    }
}

/// The region as a grid of cells, with the cells already taken by placed parts. The grid has `gap` extra cells all round, so a
/// part's thickened mask can overhang the edge without leaving it.
struct Grid {
    cols: usize,
    rows: usize,
    gap: usize,
    stride: usize,
    taken: Vec<bool>,
}

impl Grid {
    fn new(cols: usize, rows: usize, gap: usize) -> Grid {
        let stride = cols + 2 * gap;
        Grid { cols, rows, gap, stride, taken: vec![false; stride * (rows + 2 * gap)] }
    }

    /// The first spot, scanning along each row from the top-left, where `probe` touches no taken cell, with the part's bottom
    /// edge no lower than row `limit`. Adds the cells it inspected to `work` and gives up when that passes [`MAX_WORK`].
    fn first_fit(&self, mask: &Mask, probe: &[(usize, usize)], limit: usize, work: &mut u64) -> Option<(usize, usize)> {
        if mask.cols > self.cols || mask.rows > self.rows {
            return None;
        }
        for top in 0..=(self.rows - mask.rows) {
            if top + mask.rows > limit {
                return None;
            }
            for left in 0..=(self.cols - mask.cols) {
                let mut inspected = 1_u64;
                let clear = probe.iter().all(|&(x, y)| {
                    inspected += 1;
                    !self.taken.get((top + y) * self.stride + left + x).copied().unwrap_or(false)
                });
                *work = work.saturating_add(inspected);
                if clear {
                    return Some((left, top));
                }
                if *work > MAX_WORK {
                    return None;
                }
            }
        }
        None
    }

    fn occupy(&mut self, mask: &Mask, left: usize, top: usize) {
        for y in 0..mask.rows {
            for x in 0..mask.cols {
                if mask.get(x, y) {
                    let at = (top + self.gap + y) * self.stride + left + self.gap + x;
                    if let Some(c) = self.taken.get_mut(at) {
                        *c = true;
                    }
                }
            }
        }
    }
}
