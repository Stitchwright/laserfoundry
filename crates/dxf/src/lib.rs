//! ASCII DXF import, written from the public DXF reference. Supports LINE, LWPOLYLINE (with bulge
//! arcs), POLYLINE/VERTEX, CIRCLE, ARC, ELLIPSE, SPLINE (as a sampled curve) and INSERT of block
//! definitions (placed, scaled and rotated; nested up to a few levels). Unsupported entities are
//! reported, never fatal.

use std::collections::BTreeMap;

use kurbo::{Affine, Arc, Shape, SvgArc, Vec2};
use laserfoundry_geom::{BezPath, Point, bounds_of_polylines, ellipse_path, flatten};
use thiserror::Error;

pub const MAX_DXF_BYTES: usize = 100 * 1024 * 1024;
pub const MAX_ENTITIES: usize = 500_000;

#[derive(Debug, Error, PartialEq)]
pub enum DxfError {
    #[error("DXF is larger than {} MB", MAX_DXF_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("not an ASCII DXF file (binary DXF and DWG are not supported)")]
    NotAscii,
    #[error("the file has no ENTITIES section")]
    NoEntities,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DxfPath {
    pub layer: String,
    /// mm, y down, with the drawing's top-left corner moved to the origin.
    pub path: BezPath,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DxfImport {
    pub paths: Vec<DxfPath>,
    pub width_mm: f64,
    pub height_mm: f64,
    /// Entity types that were present but not imported, with counts.
    pub skipped: BTreeMap<String, usize>,
}

pub fn import_dxf(data: &[u8]) -> Result<DxfImport, DxfError> {
    if data.len() > MAX_DXF_BYTES {
        return Err(DxfError::TooLarge);
    }
    if data.starts_with(b"AutoCAD Binary DXF") || data.contains(&0) {
        return Err(DxfError::NotAscii);
    }
    let text = String::from_utf8_lossy(data);
    let pairs = pairs(&text);
    let scale = unit_scale(&pairs);
    let start = pairs.iter().position(|(c, v)| *c == 2 && v == "ENTITIES").ok_or(DxfError::NoEntities)?;

    let entities = collect_entities(&pairs, start + 1);
    let blocks = collect_blocks(&pairs);
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut paths = Vec::new();
    let mut budget = MAX_ENTITIES;
    convert(&entities, Affine::IDENTITY, 0, &blocks, &mut paths, &mut skipped, &mut budget);

    // DXF is y-up in drawing units: scale to mm, flip to y-down, move the top-left to the origin.
    let flip = Affine::scale_non_uniform(scale, -scale);
    for p in &mut paths {
        p.path.apply_affine(flip);
    }
    let all: Vec<_> = paths.iter().flat_map(|p| flatten(&p.path, 0.05)).collect();
    let (width_mm, height_mm) = match bounds_of_polylines(&all) {
        Some(b) => {
            let shift = Affine::translate((-b.x0, -b.y0));
            for p in &mut paths {
                p.path.apply_affine(shift);
            }
            (b.width(), b.height())
        }
        None => (0.0, 0.0),
    };
    Ok(DxfImport { paths, width_mm, height_mm, skipped })
}

/// Entities from `from` up to the end of the section, with POLYLINE vertices left as separate entities.
fn collect_entities(pairs: &[(i32, String)], from: usize) -> Vec<Entity> {
    let mut entities: Vec<Entity> = Vec::new();
    let mut current: Option<Entity> = None;
    for (code, value) in pairs.iter().skip(from) {
        if *code == 0 {
            if let Some(e) = current.take() {
                entities.push(e);
            }
            if value == "ENDSEC" || entities.len() >= MAX_ENTITIES {
                break;
            }
            current = Some(Entity { kind: value.clone(), data: Vec::new() });
            continue;
        }
        if let Some(e) = current.as_mut() {
            e.data.push((*code, value.clone()));
        }
    }
    entities
}

/// Block definitions: name → (base point, entities).
type Blocks = std::collections::HashMap<String, ((f64, f64), Vec<Entity>)>;

/// Most block definitions read, and how deeply inserts may nest.
const MAX_BLOCKS: usize = 10_000;
const MAX_INSERT_DEPTH: usize = 4;

fn collect_blocks(pairs: &[(i32, String)]) -> Blocks {
    let mut blocks = Blocks::new();
    let Some(start) = pairs.iter().position(|(c, v)| *c == 2 && v == "BLOCKS") else { return blocks };
    let all = collect_entities(pairs, start + 1);
    let mut current: Option<(String, (f64, f64), Vec<Entity>)> = None;
    for e in all {
        match e.kind.as_str() {
            "BLOCK" => {
                let name = e.text(2).unwrap_or("").to_string();
                current = Some((name, (e.num(10).unwrap_or(0.0), e.num(20).unwrap_or(0.0)), Vec::new()));
            }
            "ENDBLK" => {
                if let Some((name, base, ents)) = current.take()
                    && blocks.len() < MAX_BLOCKS
                {
                    blocks.insert(name, (base, ents));
                }
            }
            _ => {
                if let Some((_, _, ents)) = current.as_mut() {
                    ents.push(e);
                }
            }
        }
    }
    blocks
}

/// Turns entities into paths, placing them with `xf`; `budget` bounds the total number built so a
/// block that inserts itself (or a hostile pile of inserts) cannot run away.
fn convert(
    entities: &[Entity],
    xf: Affine,
    depth: usize,
    blocks: &Blocks,
    out: &mut Vec<DxfPath>,
    skipped: &mut BTreeMap<String, usize>,
    budget: &mut usize,
) {
    let mut i = 0;
    while let Some(e) = entities.get(i) {
        i += 1;
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let layer = e.text(8).unwrap_or("0").to_string();
        let built = match e.kind.as_str() {
            "LINE" => build_line(e),
            "LWPOLYLINE" => build_lwpolyline(e),
            "CIRCLE" => build_circle(e),
            "ARC" => build_arc(e),
            "ELLIPSE" => build_ellipse(e),
            "SPLINE" => {
                let built = build_spline(e);
                if built.is_none() {
                    *skipped.entry("SPLINE".to_string()).or_default() += 1;
                }
                built
            }
            "INSERT" => {
                let name = e.text(2).unwrap_or("");
                match blocks.get(name) {
                    Some((base, inner)) if depth < MAX_INSERT_DEPTH => {
                        let (sx, sy) = (e.num(41).unwrap_or(1.0), e.num(42).unwrap_or(1.0));
                        let (x, y) = (e.num(10).unwrap_or(0.0), e.num(20).unwrap_or(0.0));
                        if [sx, sy, x, y].iter().all(|v| v.abs() < 1e9) {
                            let place = Affine::translate((x, y))
                                * Affine::rotate(e.num(50).unwrap_or(0.0).to_radians())
                                * Affine::scale_non_uniform(sx, sy)
                                * Affine::translate((-base.0, -base.1));
                            convert(inner, xf * place, depth + 1, blocks, out, skipped, budget);
                        }
                    }
                    _ => *skipped.entry("INSERT".to_string()).or_default() += 1,
                }
                None
            }
            "POLYLINE" => {
                let closed = e.int(70).unwrap_or(0) & 1 == 1;
                let mut verts = Vec::new();
                while let Some(v) = entities.get(i) {
                    i += 1;
                    if v.kind == "SEQEND" {
                        break;
                    }
                    if v.kind == "VERTEX"
                        && let (Some(x), Some(y)) = (v.num(10), v.num(20))
                    {
                        verts.push((x, y, v.num(42).unwrap_or(0.0)));
                    }
                }
                polyline_from_vertices(&verts, closed)
            }
            "VERTEX" | "SEQEND" | "ENDBLK" | "BLOCK" => None,
            other => {
                *skipped.entry(other.to_string()).or_default() += 1;
                None
            }
        };
        if let Some(mut path) = built {
            path.apply_affine(xf);
            out.push(DxfPath { layer, path });
        }
    }
}

struct Entity {
    kind: String,
    data: Vec<(i32, String)>,
}

impl Entity {
    fn text(&self, code: i32) -> Option<&str> {
        self.data.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_str())
    }

    fn num(&self, code: i32) -> Option<f64> {
        self.text(code).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
    }

    fn int(&self, code: i32) -> Option<i64> {
        self.text(code).and_then(|v| v.trim().parse::<i64>().ok())
    }

    fn nums(&self, code: i32) -> impl Iterator<Item = f64> + '_ {
        self.data.iter().filter(move |(c, _)| *c == code).filter_map(|(_, v)| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
    }
}

fn pairs(text: &str) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        if let Ok(code) = code.trim().parse::<i32>() {
            out.push((code, value.trim().to_string()));
        }
    }
    out
}

/// `$INSUNITS` → mm per drawing unit. Unitless (0) is treated as mm.
fn unit_scale(pairs: &[(i32, String)]) -> f64 {
    let units = pairs
        .iter()
        .position(|(c, v)| *c == 9 && v == "$INSUNITS")
        .and_then(|i| pairs.get(i + 1))
        .and_then(|(_, v)| v.parse::<i32>().ok())
        .unwrap_or(0);
    match units {
        1 => 25.4,
        2 => 304.8,
        3 => 1_609_344.0,
        5 => 10.0,
        6 => 1000.0,
        7 => 1_000_000.0,
        8 => 25.4e-6,
        9 => 0.0254,
        10 => 914.4,
        _ => 1.0,
    }
}

fn build_line(e: &Entity) -> Option<BezPath> {
    let mut p = BezPath::new();
    p.move_to((e.num(10)?, e.num(20)?));
    p.line_to((e.num(11)?, e.num(21)?));
    Some(p)
}

fn build_circle(e: &Entity) -> Option<BezPath> {
    let r = e.num(40)?;
    if r <= 0.0 {
        return None;
    }
    Some(ellipse_path(e.num(10)?, e.num(20)?, r, r))
}

fn build_arc(e: &Entity) -> Option<BezPath> {
    let r = e.num(40)?;
    if r <= 0.0 {
        return None;
    }
    let start = e.num(50)?.to_radians();
    let mut sweep = (e.num(51)?.to_radians() - start).rem_euclid(std::f64::consts::TAU);
    if sweep == 0.0 {
        sweep = std::f64::consts::TAU;
    }
    let arc = Arc::new((e.num(10)?, e.num(20)?), Vec2::new(r, r), start, sweep, 0.0);
    Some(arc.to_path(0.05))
}

fn build_ellipse(e: &Entity) -> Option<BezPath> {
    let (cx, cy) = (e.num(10)?, e.num(20)?);
    let major = Vec2::new(e.num(11)?, e.num(21)?);
    let ratio = e.num(40)?;
    if major.hypot() < 1e-12 || !(ratio > 0.0 && ratio <= 1.0) {
        return None;
    }
    let (start, end) = (e.num(41).unwrap_or(0.0), e.num(42).unwrap_or(std::f64::consts::TAU));
    let mut sweep = (end - start).rem_euclid(std::f64::consts::TAU);
    if sweep < 1e-9 {
        sweep = std::f64::consts::TAU;
    }
    let arc = Arc::new((cx, cy), Vec2::new(major.hypot(), major.hypot() * ratio), start, sweep, major.atan2());
    let mut path = arc.to_path(0.05);
    if sweep >= std::f64::consts::TAU - 1e-9 {
        path.close_path();
    }
    Some(path)
}

/// Samples per knot span and most control points taken from one spline.
const SPLINE_SAMPLES: usize = 24;
const MAX_SPLINE_POINTS: usize = 100_000;
const MAX_SPLINE_WORK: usize = 50_000_000;

/// A B-spline (NURBS when weights are given) evaluated with de Boor's algorithm; with no control
/// points the fit points are joined by straight lines instead.
fn build_spline(e: &Entity) -> Option<BezPath> {
    let closed = e.int(70).unwrap_or(0) & 1 == 1;
    let xs: Vec<f64> = e.nums(10).collect();
    let ys: Vec<f64> = e.nums(20).collect();
    let n = xs.len().min(ys.len());
    if n > MAX_SPLINE_POINTS {
        return None;
    }
    let degree = usize::try_from(e.int(71).unwrap_or(3)).ok().filter(|d| (1..=10).contains(d)).unwrap_or(3);
    let knots: Vec<f64> = e.nums(40).collect();
    let weights: Vec<f64> = e.nums(41).collect();
    if n > degree && knots.len() == n + degree + 1 && knots.windows(2).all(|w| w[0] <= w[1]) {
        let (lo, hi) = (knots[degree], knots[n]);
        if hi - lo > 1e-12 {
            let steps = ((n - degree) * SPLINE_SAMPLES).clamp(8, MAX_SPLINE_POINTS);
            // Evaluation cost grows with points x samples x 2^degree; very heavy curves use their polygon.
            if n.saturating_mul(steps).saturating_mul(1 << degree) > MAX_SPLINE_WORK {
                return build_polygon(&xs, &ys, closed);
            }
            let w = |i: usize| weights.get(i).copied().filter(|v| *v > 0.0).unwrap_or(1.0);
            let mut path = BezPath::new();
            for k in 0..=steps {
                let u = if k == steps { hi } else { lo + (hi - lo) * k as f64 / steps as f64 };
                let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
                for i in 0..n {
                    let b = basis(i, degree, u, &knots, hi);
                    if b != 0.0 {
                        let bw = b * w(i);
                        sx += bw * xs[i];
                        sy += bw * ys[i];
                        sw += bw;
                    }
                }
                if sw.abs() < 1e-12 {
                    return None;
                }
                let pt = (sx / sw, sy / sw);
                if k == 0 { path.move_to(pt) } else { path.line_to(pt) }
            }
            if closed {
                path.close_path();
            }
            return Some(path);
        }
    }
    // Fit points (codes 11/21), or the control polygon when the knots are unusable.
    let (fx, fy): (Vec<f64>, Vec<f64>) = if e.nums(11).count() >= 2 { (e.nums(11).collect(), e.nums(21).collect()) } else { (xs, ys) };
    build_polygon(&fx, &fy, closed)
}

fn build_polygon(xs: &[f64], ys: &[f64], closed: bool) -> Option<BezPath> {
    let m = xs.len().min(ys.len());
    if !(2..=MAX_SPLINE_POINTS).contains(&m) {
        return None;
    }
    let mut path = BezPath::new();
    path.move_to((xs[0], ys[0]));
    for i in 1..m {
        path.line_to((xs[i], ys[i]));
    }
    if closed {
        path.close_path();
    }
    Some(path)
}

/// The i-th B-spline basis function of the given degree at `u` (Cox-de Boor recursion). The last
/// span includes its end point.
fn basis(i: usize, degree: usize, u: f64, knots: &[f64], end: f64) -> f64 {
    if degree == 0 {
        let (a, b) = (knots[i], knots[i + 1]);
        let inside = (u >= a && u < b) || (u == end && a < b && b == end);
        return if inside { 1.0 } else { 0.0 };
    }
    let mut v = 0.0;
    let d1 = knots[i + degree] - knots[i];
    if d1 > 0.0 {
        v += (u - knots[i]) / d1 * basis(i, degree - 1, u, knots, end);
    }
    let d2 = knots[i + degree + 1] - knots[i + 1];
    if d2 > 0.0 {
        v += (knots[i + degree + 1] - u) / d2 * basis(i + 1, degree - 1, u, knots, end);
    }
    v
}

fn build_lwpolyline(e: &Entity) -> Option<BezPath> {
    let closed = e.int(70).unwrap_or(0) & 1 == 1;
    let xs: Vec<f64> = e.nums(10).collect();
    let ys: Vec<f64> = e.nums(20).collect();
    // Bulges (code 42) are per-vertex but only present when non-zero, so rebuild by vertex order.
    let mut bulges = vec![0.0; xs.len().min(ys.len())];
    let mut vertex: isize = -1;
    for (code, v) in &e.data {
        match code {
            10 => vertex += 1,
            42 => {
                if let (Ok(b), Some(slot)) = (v.trim().parse::<f64>(), usize::try_from(vertex).ok().and_then(|i| bulges.get_mut(i))) {
                    *slot = b;
                }
            }
            _ => {}
        }
    }
    let verts: Vec<(f64, f64, f64)> = xs.iter().zip(&ys).zip(&bulges).map(|((x, y), b)| (*x, *y, *b)).collect();
    polyline_from_vertices(&verts, closed)
}

fn polyline_from_vertices(verts: &[(f64, f64, f64)], closed: bool) -> Option<BezPath> {
    if verts.len() < 2 {
        return None;
    }
    let mut path = BezPath::new();
    let (x0, y0, _) = *verts.first()?;
    path.move_to((x0, y0));
    let segments = if closed { verts.len() } else { verts.len() - 1 };
    for i in 0..segments {
        let (ax, ay, bulge) = *verts.get(i)?;
        let (bx, by, _) = *verts.get((i + 1) % verts.len())?;
        append_segment(&mut path, Point::new(ax, ay), Point::new(bx, by), bulge);
    }
    if closed {
        path.close_path();
    }
    Some(path)
}

fn append_segment(path: &mut BezPath, a: Point, b: Point, bulge: f64) {
    let chord = a.distance(b);
    if bulge.abs() < 1e-9 || chord < 1e-12 {
        path.line_to(b);
        return;
    }
    let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let svg = SvgArc { from: a, to: b, radii: Vec2::new(radius, radius), x_rotation: 0.0, large_arc: bulge.abs() > 1.0, sweep: bulge > 0.0 };
    match Arc::from_svg_arc(&svg) {
        Some(arc) => {
            for el in arc.append_iter(0.05) {
                path.push(el);
            }
        }
        None => path.line_to(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dxf(entities: &str, units: i32) -> String {
        format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{units}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n")
    }

    fn bounds(imp: &DxfImport) -> kurbo::Rect {
        bounds_of_polylines(&imp.paths.iter().flat_map(|p| flatten(&p.path, 0.01)).collect::<Vec<_>>()).unwrap()
    }

    mod entity_tests {
        use super::*;

        #[test]
        fn line_and_circle_import_in_mm() {
            let src = dxf("0\nLINE\n8\nCUT\n10\n0\n20\n0\n11\n30\n21\n10\n0\nCIRCLE\n10\n50\n20\n5\n40\n5\n", 4);
            let imp = import_dxf(src.as_bytes()).unwrap();
            assert_eq!(imp.paths.len(), 2);
            assert_eq!(imp.paths[0].layer, "CUT");
            assert!((imp.width_mm - 55.0).abs() < 0.01, "{}", imp.width_mm);
            assert!((imp.height_mm - 10.0).abs() < 0.01, "{}", imp.height_mm);
        }

        #[test]
        fn inches_are_scaled_to_mm() {
            let src = dxf("0\nLINE\n10\n0\n20\n0\n11\n2\n21\n0\n", 1);
            assert!((import_dxf(src.as_bytes()).unwrap().width_mm - 50.8).abs() < 1e-9);
        }

        #[test]
        fn closed_lwpolyline_with_a_bulge_makes_a_half_circle_cap() {
            // Square 10x10 whose top edge bulges into a semicircle (bulge 1 = 180 degrees).
            let src = dxf("0\nLWPOLYLINE\n70\n1\n90\n4\n10\n0\n20\n0\n10\n10\n20\n0\n10\n10\n20\n10\n42\n1\n10\n0\n20\n10\n", 4);
            let imp = import_dxf(src.as_bytes()).unwrap();
            assert!((imp.height_mm - 15.0).abs() < 0.05, "{}", imp.height_mm);
        }

        #[test]
        fn arc_spans_the_requested_angles() {
            let src = dxf("0\nARC\n10\n0\n20\n0\n40\n10\n50\n0\n51\n90\n", 4);
            let b = bounds(&import_dxf(src.as_bytes()).unwrap());
            assert!((b.width() - 10.0).abs() < 0.05 && (b.height() - 10.0).abs() < 0.05, "{b:?}");
        }

        #[test]
        fn old_style_polyline_is_supported() {
            let src = dxf("0\nPOLYLINE\n70\n0\n0\nVERTEX\n10\n0\n20\n0\n0\nVERTEX\n10\n10\n20\n5\n0\nSEQEND\n", 4);
            assert_eq!(import_dxf(src.as_bytes()).unwrap().paths.len(), 1);
        }

        #[test]
        fn unsupported_entities_are_reported_not_fatal() {
            let src = dxf("0\nSPLINE\n8\n0\n0\nTEXT\n1\nhi\n", 4); // a spline without points cannot be drawn
            let imp = import_dxf(src.as_bytes()).unwrap();
            assert!(imp.paths.is_empty());
            assert_eq!(imp.skipped.get("SPLINE"), Some(&1));
            assert_eq!(imp.skipped.get("TEXT"), Some(&1));
        }
    }

    mod curve_and_block_tests {
        use super::*;

        #[test]
        fn a_full_ellipse_and_a_rotated_arc_have_the_right_extent() {
            let src = dxf("0\nELLIPSE\n10\n0\n20\n0\n11\n20\n21\n0\n40\n0.5\n41\n0\n42\n6.283185307179586\n", 4);
            let b = bounds(&import_dxf(src.as_bytes()).unwrap());
            assert!((b.width() - 40.0).abs() < 0.1 && (b.height() - 20.0).abs() < 0.1, "{b:?}");
            // The major axis along +y: the same ellipse stood on end.
            let tall = dxf("0\nELLIPSE\n10\n0\n20\n0\n11\n0\n21\n20\n40\n0.5\n41\n0\n42\n6.283185307179586\n", 4);
            let b = bounds(&import_dxf(tall.as_bytes()).unwrap());
            assert!((b.width() - 20.0).abs() < 0.1 && (b.height() - 40.0).abs() < 0.1, "{b:?}");
            assert!(import_dxf(dxf("0\nELLIPSE\n10\n0\n20\n0\n11\n0\n21\n0\n40\n0.5\n", 4).as_bytes()).unwrap().paths.is_empty());
            assert!(
                import_dxf(dxf("0\nELLIPSE\n10\n0\n20\n0\n11\n5\n21\n0\n40\n3\n", 4).as_bytes()).unwrap().paths.is_empty(),
                "ratio above 1 is refused"
            );
        }

        #[test]
        fn a_cubic_spline_passes_through_its_end_points_and_stays_inside_its_control_polygon() {
            // Degree 3, four control points, clamped knots: a single Bezier curve.
            let spline = "0\nSPLINE\n70\n8\n71\n3\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n10\n10\n20\n30\n10\n20\n20\n30\n10\n30\n20\n0\n";
            let imp = import_dxf(dxf(spline, 4).as_bytes()).unwrap();
            assert_eq!(imp.paths.len(), 1);
            let b = bounds(&imp);
            assert!((b.width() - 30.0).abs() < 0.1, "reaches both ends: {b:?}");
            assert!(b.height() > 15.0 && b.height() <= 30.0 + 1e-6, "inside the control polygon: {b:?}");
        }

        #[test]
        fn splines_without_usable_knots_fall_back_to_their_points_and_junk_is_skipped() {
            let fit = "0\nSPLINE\n11\n0\n21\n0\n11\n10\n21\n5\n11\n20\n21\n0\n";
            assert_eq!(import_dxf(dxf(fit, 4).as_bytes()).unwrap().paths.len(), 1);
            let bad_knots = "0\nSPLINE\n71\n3\n40\n5\n40\n1\n10\n0\n20\n0\n10\n1\n20\n1\n";
            let imp = import_dxf(dxf(bad_knots, 4).as_bytes()).unwrap();
            assert_eq!(imp.paths.len(), 1, "the control polygon is used instead");
            assert!(import_dxf(dxf("0\nSPLINE\n71\n-3\n10\n0\n20\n0\n", 4).as_bytes()).unwrap().paths.is_empty());
        }

        fn with_blocks(blocks: &str, entities: &str) -> String {
            format!("0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n")
        }

        #[test]
        fn inserted_blocks_are_placed_scaled_and_rotated() {
            let square = "0\nBLOCK\n2\nSQ\n10\n0\n20\n0\n0\nLINE\n10\n0\n20\n0\n11\n10\n21\n0\n0\nLINE\n10\n10\n20\n0\n11\n10\n21\n5\n0\nENDBLK\n";
            let one = import_dxf(with_blocks(square, "0\nINSERT\n2\nSQ\n10\n100\n20\n50\n").as_bytes()).unwrap();
            assert_eq!(one.paths.len(), 2);
            let b = bounds(&one);
            assert!((b.width() - 10.0).abs() < 1e-6 && (b.height() - 5.0).abs() < 1e-6, "{b:?}");
            let scaled = import_dxf(with_blocks(square, "0\nINSERT\n2\nSQ\n10\n0\n20\n0\n41\n3\n42\n2\n").as_bytes()).unwrap();
            let b = bounds(&scaled);
            assert!((b.width() - 30.0).abs() < 1e-6 && (b.height() - 10.0).abs() < 1e-6, "{b:?}");
            let turned = import_dxf(with_blocks(square, "0\nINSERT\n2\nSQ\n10\n0\n20\n0\n50\n90\n").as_bytes()).unwrap();
            let b = bounds(&turned);
            assert!((b.width() - 5.0).abs() < 1e-6 && (b.height() - 10.0).abs() < 1e-6, "{b:?}");
            let twice = import_dxf(with_blocks(square, "0\nINSERT\n2\nSQ\n10\n0\n20\n0\n0\nINSERT\n2\nSQ\n10\n20\n20\n0\n").as_bytes()).unwrap();
            assert_eq!(twice.paths.len(), 4);
        }

        #[test]
        fn unknown_hostile_and_self_inserting_blocks_are_bounded() {
            let missing = import_dxf(with_blocks("", "0\nINSERT\n2\nNOPE\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n").as_bytes()).unwrap();
            assert_eq!(missing.skipped.get("INSERT"), Some(&1));
            let recursive = "0\nBLOCK\n2\nR\n10\n0\n20\n0\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n0\nINSERT\n2\nR\n0\nENDBLK\n";
            let imp = import_dxf(with_blocks(recursive, "0\nINSERT\n2\nR\n").as_bytes()).unwrap();
            assert!(imp.paths.len() <= MAX_INSERT_DEPTH + 1, "{}", imp.paths.len());
            let huge = import_dxf(
                with_blocks("0\nBLOCK\n2\nB\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n0\nENDBLK\n", "0\nINSERT\n2\nB\n41\n1e30\n42\n1e30\n").as_bytes(),
            )
            .unwrap();
            assert!(huge.paths.is_empty(), "absurd scales are ignored");
        }
    }

    mod hostile_tests {
        use super::*;

        #[test]
        fn garbage_does_not_panic() {
            assert_eq!(import_dxf(b"hello"), Err(DxfError::NoEntities));
            assert_eq!(import_dxf(b"\0\0\0"), Err(DxfError::NotAscii));
            assert!(import_dxf(b"AutoCAD Binary DXF\r\n\x1a\0").is_err());
        }

        #[test]
        fn nan_and_missing_fields_are_skipped() {
            let src = dxf("0\nLINE\n10\nNaN\n20\n0\n11\n1\n21\n1\n0\nCIRCLE\n10\n0\n", 4);
            assert!(import_dxf(src.as_bytes()).unwrap().paths.is_empty());
        }

        #[test]
        fn zero_radius_circle_is_skipped() {
            let src = dxf("0\nCIRCLE\n10\n0\n20\n0\n40\n0\n", 4);
            assert!(import_dxf(src.as_bytes()).unwrap().paths.is_empty());
        }
    }
}
