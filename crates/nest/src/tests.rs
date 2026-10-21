use std::time::Instant;

use super::*;

/// A closed rectangle part with its top-left corner at (x, y).
fn rect(x: f64, y: f64, w: f64, h: f64) -> Part {
    let points = vec![Point::new(x, y), Point::new(x + w, y), Point::new(x + w, y + h), Point::new(x, y + h)];
    Part { outlines: vec![Polyline::new(points, true)] }
}

fn line(a: (f64, f64), b: (f64, f64)) -> Part {
    Part { outlines: vec![Polyline::new(vec![Point::new(a.0, a.1), Point::new(b.0, b.1)], false)] }
}

fn region(w: f64, h: f64) -> Rect {
    Rect::new(0.0, 0.0, w, h)
}

/// The part's outlines after the placement is applied.
fn moved(part: &Part, placement: &Placement) -> Vec<Polyline> {
    let m = placement.affine();
    part.outlines.iter().map(|l| Polyline::new(l.points.iter().map(|p| m * *p).collect(), l.closed)).collect()
}

fn box_of(lines: &[Polyline]) -> Rect {
    laserfoundry_geom::bounds_of_polylines(lines).unwrap()
}

// ---- independent geometry, so the tests do not lean on the code under test ----

fn cross(o: Point, a: Point, b: Point) -> f64 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn point_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0) };
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

fn inside(p: Point, poly: &Polyline) -> bool {
    if !poly.closed {
        return false;
    }
    let pts = &poly.points;
    let mut hit = false;
    for k in 0..pts.len() {
        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            hit = !hit;
        }
    }
    hit
}

fn edges(poly: &Polyline) -> Vec<(Point, Point)> {
    let pts = &poly.points;
    let n = if poly.closed { pts.len() } else { pts.len().saturating_sub(1) };
    (0..n).map(|k| (pts[k], pts[(k + 1) % pts.len()])).collect()
}

/// The least distance between two sets of outlines; zero if they cross or one lies inside a closed outline of the other.
fn distance(a: &[Polyline], b: &[Polyline]) -> f64 {
    let mut least = f64::INFINITY;
    for pa in a {
        for pb in b {
            if pa.points.iter().any(|p| inside(*p, pb)) || pb.points.iter().any(|p| inside(*p, pa)) {
                return 0.0;
            }
            for (a0, a1) in edges(pa) {
                for (b0, b1) in edges(pb) {
                    if segments_cross(a0, a1, b0, b1) {
                        return 0.0;
                    }
                    least = least
                        .min(point_segment(a0, b0, b1))
                        .min(point_segment(a1, b0, b1))
                        .min(point_segment(b0, a0, a1))
                        .min(point_segment(b1, a0, a1));
                }
            }
        }
    }
    least
}

/// Checks everything the nesting promises about `result`, on the real outlines.
fn check(parts: &[Part], result: &Nesting, region: Rect, options: &Options) {
    let mut all: Vec<usize> = result.placed.iter().map(|p| p.part).chain(result.unplaced.iter().copied()).collect();
    all.sort_unstable();
    assert_eq!(all, (0..parts.len()).collect::<Vec<_>>(), "every part is placed or reported, once");
    let inner = Rect::new(region.x0 + options.margin, region.y0 + options.margin, region.x1 - options.margin, region.y1 - options.margin);
    let placed: Vec<Vec<Polyline>> = result.placed.iter().map(|p| moved(&parts[p.part], p)).collect();
    for (k, lines) in placed.iter().enumerate() {
        let b = box_of(lines);
        assert!(
            b.x0 >= inner.x0 - 1e-6 && b.y0 >= inner.y0 - 1e-6 && b.x1 <= inner.x1 + 1e-6 && b.y1 <= inner.y1 + 1e-6,
            "part {k} leaves the region: {b:?} in {inner:?}"
        );
    }
    for i in 0..placed.len() {
        for j in (i + 1)..placed.len() {
            let d = distance(&placed[i], &placed[j]);
            assert!(d > 0.0, "parts {i} and {j} touch or overlap");
            assert!(d >= options.gap - 1e-6, "parts {i} and {j} are {d} mm apart, less than the gap {}", options.gap);
        }
    }
}

mod packing_tests {
    use super::*;

    #[test]
    fn squares_that_exactly_fit_are_all_placed_without_touching() {
        let parts: Vec<Part> = (0..9).map(|_| rect(0.0, 0.0, 10.0, 10.0)).collect();
        let options = Options { gap: 0.0, rotations: 1, ..Options::default() };
        let result = nest(&parts, region(32.0, 32.0), &options).unwrap();
        assert_eq!(result.placed.len(), 9, "{:?}", result.unplaced);
        check(&parts, &result, region(32.0, 32.0), &options);
    }

    #[test]
    fn the_gap_between_parts_is_kept() {
        let sizes =
            [(12.0, 8.0), (20.0, 15.0), (7.0, 7.0), (30.0, 10.0), (9.0, 25.0), (14.0, 14.0), (5.0, 18.0), (22.0, 6.0), (11.0, 11.0), (16.0, 9.0)];
        let parts: Vec<Part> = sizes.iter().map(|&(w, h)| rect(40.0, 40.0, w, h)).collect();
        for gap in [0.5, 3.0, 6.0] {
            let options = Options { gap, ..Options::default() };
            let result = nest(&parts, region(120.0, 90.0), &options).unwrap();
            assert!(result.placed.len() >= 8, "gap {gap}: {} placed", result.placed.len());
            check(&parts, &result, region(120.0, 90.0), &options);
        }
    }

    #[test]
    fn turning_parts_lets_a_long_one_fit() {
        let parts = vec![rect(0.0, 0.0, 60.0, 10.0)];
        let fixed = nest(&parts, region(20.0, 70.0), &Options { rotations: 1, ..Options::default() }).unwrap();
        assert_eq!((fixed.placed.len(), fixed.unplaced.clone()), (0, vec![0]), "60 mm cannot lie across a 20 mm region");
        let options = Options::default();
        let turned = nest(&parts, region(20.0, 70.0), &options).unwrap();
        assert_eq!(turned.placed.len(), 1);
        let quarter = (turned.placed[0].rotation.rem_euclid(std::f64::consts::PI) - std::f64::consts::FRAC_PI_2).abs();
        assert!(quarter < 1e-9, "turned a quarter: {}", turned.placed[0].rotation);
        check(&parts, &turned, region(20.0, 70.0), &options);
    }

    #[test]
    fn parts_that_do_not_fit_are_reported_and_the_rest_are_placed() {
        let parts: Vec<Part> = (0..10).map(|_| rect(0.0, 0.0, 20.0, 20.0)).collect();
        let options = Options { gap: 0.0, rotations: 1, ..Options::default() };
        let result = nest(&parts, region(50.0, 50.0), &options).unwrap();
        assert_eq!((result.placed.len(), result.unplaced.len()), (4, 6));
        assert!(!result.stopped_early);
        check(&parts, &result, region(50.0, 50.0), &options);
    }

    #[test]
    fn the_margin_keeps_parts_off_the_edge() {
        let parts: Vec<Part> = (0..4).map(|_| rect(0.0, 0.0, 10.0, 10.0)).collect();
        let options = Options { margin: 7.0, gap: 1.0, ..Options::default() };
        let result = nest(&parts, region(60.0, 60.0), &options).unwrap();
        assert_eq!(result.placed.len(), 4);
        check(&parts, &result, region(60.0, 60.0), &options);
        let nearest = result.placed.iter().map(|p| box_of(&moved(&parts[p.part], p)).x0).fold(f64::INFINITY, f64::min);
        assert!(nearest >= 7.0 - 1e-9, "{nearest}");
    }

    #[test]
    fn the_region_need_not_start_at_the_origin() {
        let parts: Vec<Part> = (0..3).map(|_| rect(500.0, 500.0, 10.0, 10.0)).collect();
        let area = Rect::new(100.0, 200.0, 140.0, 230.0);
        let result = nest(&parts, area, &Options::default()).unwrap();
        assert_eq!(result.placed.len(), 3);
        check(&parts, &result, area, &Options::default());
    }

    #[test]
    fn lines_nest_too_and_take_only_their_own_room() {
        let parts: Vec<Part> = (0..5).map(|_| line((0.0, 0.0), (50.0, 0.0))).collect();
        let options = Options { rotations: 1, ..Options::default() };
        let result = nest(&parts, region(60.0, 30.0), &options).unwrap();
        assert_eq!(result.placed.len(), 5);
        check(&parts, &result, region(60.0, 30.0), &options);
    }

    #[test]
    fn a_part_made_of_several_outlines_moves_as_one() {
        let two = Part { outlines: rect(0.0, 0.0, 10.0, 10.0).outlines.into_iter().chain(rect(30.0, 0.0, 10.0, 10.0).outlines).collect() };
        let parts = vec![two.clone(), two];
        let options = Options { rotations: 1, ..Options::default() };
        let result = nest(&parts, region(100.0, 40.0), &options).unwrap();
        assert_eq!(result.placed.len(), 2);
        for p in &result.placed {
            let lines = moved(&parts[p.part], p);
            assert!((lines[1].points[0].x - lines[0].points[0].x - 30.0).abs() < 1e-9, "the two outlines keep their distance");
        }
        check(&parts, &result, region(100.0, 40.0), &options);
    }

    #[test]
    fn the_same_input_gives_the_same_answer() {
        let parts: Vec<Part> = (0..12).map(|i| rect(0.0, 0.0, 6.0 + f64::from(i % 5) * 3.0, 5.0 + f64::from(i % 3) * 4.0)).collect();
        let a = nest(&parts, region(80.0, 60.0), &Options::default()).unwrap();
        let b = nest(&parts, region(80.0, 60.0), &Options::default()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn random_shapes_never_overlap_or_leave_the_region() {
        let mut seed = 99_u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f64) / f64::from(1u32 << 31)
        };
        for round in 0..6 {
            let parts: Vec<Part> = (0..14)
                .map(|_| {
                    let n = 5 + (next() * 5.0) as usize;
                    let base = 4.0 + next() * 10.0;
                    let points: Vec<Point> = (0..n)
                        .map(|k| {
                            let a = std::f64::consts::TAU * k as f64 / n as f64;
                            let r = base * (0.5 + next() * 0.5);
                            Point::new(50.0 + r * a.cos(), 50.0 + r * a.sin())
                        })
                        .collect();
                    Part { outlines: vec![Polyline::new(points, true)] }
                })
                .collect();
            let options =
                Options { gap: [0.0, 1.0, 2.5, 4.0][round % 4], rotations: [1, 4, 8][round % 3], margin: [0.0, 3.0][round % 2], resolution: None };
            let result = nest(&parts, region(120.0, 90.0), &options).unwrap();
            assert!(!result.placed.is_empty(), "round {round}");
            check(&parts, &result, region(120.0, 90.0), &options);
        }
    }

    #[test]
    fn a_hundred_small_parts_nest_quickly_and_all_fit() {
        let parts: Vec<Part> = (0..100).map(|_| rect(0.0, 0.0, 15.0, 15.0)).collect();
        let options = Options::default();
        let start = Instant::now();
        let result = nest(&parts, region(300.0, 200.0), &options).unwrap();
        assert_eq!(result.placed.len(), 100, "{} unplaced", result.unplaced.len());
        assert!(start.elapsed().as_secs() < 20, "took {:?}", start.elapsed());
        check(&parts, &result, region(300.0, 200.0), &options);
    }
}

mod mask_tests {
    use super::*;

    fn mask_of(part: &Part, cell: f64) -> Mask {
        let b = bounds(part).unwrap();
        Mask::of(part, b.center(), 0.0, cell, 1000, 1000).unwrap()
    }

    #[test]
    fn a_diagonal_line_marks_the_cells_it_runs_through() {
        let m = mask_of(&line((0.0, 0.0), (10.0, 10.0)), 1.0);
        for k in 0..=10 {
            assert!(m.get(k, k), "cell {k}");
        }
        // Through every corner it passes the two cells beside the path too, so nothing it touches is left out.
        assert!(m.get(1, 0) && m.get(0, 1));
    }

    #[test]
    fn a_closed_outline_marks_its_inside_and_a_hole_stays_solid() {
        let n = 64;
        let circle: Vec<Point> = (0..n)
            .map(|k| {
                Point::new(
                    20.0 + 10.0 * (std::f64::consts::TAU * f64::from(k) / f64::from(n)).cos(),
                    20.0 + 10.0 * (std::f64::consts::TAU * f64::from(k) / f64::from(n)).sin(),
                )
            })
            .collect();
        let part = Part { outlines: vec![Polyline::new(circle, true)] };
        let m = mask_of(&part, 1.0);
        let marked = m.cells.iter().filter(|c| **c).count();
        assert!((314..=400).contains(&marked), "a circle of radius 10 is about 314 cells, plus its edge: {marked}");
        assert!(m.get(10, 10), "the middle is inside");
        assert!(!m.get(0, 0), "the corner of the box is outside the circle");
    }

    #[test]
    fn the_probe_thickens_by_the_gap_in_every_direction() {
        let m = Mask { cols: 1, rows: 1, cells: vec![true], min: Point::ORIGIN };
        assert_eq!(m.probe(0), vec![(0, 0)]);
        let probe = m.probe(2);
        assert_eq!(probe.len(), 25, "a 5 by 5 block");
        assert!(probe.iter().all(|&(x, y)| x < 5 && y < 5));
    }

    #[test]
    fn a_part_too_big_for_the_grid_has_no_mask() {
        let big = rect(0.0, 0.0, 1e12, 1e12);
        assert!(Mask::of(&big, Point::ORIGIN, 0.0, 0.1, 100, 100).is_none());
    }
}

mod hostile_tests {
    use super::*;

    #[test]
    fn bad_numbers_in_a_part_are_refused() {
        let good = rect(0.0, 0.0, 5.0, 5.0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let parts = vec![good.clone(), rect(0.0, 0.0, bad, 5.0)];
            assert_eq!(nest(&parts, region(50.0, 50.0), &Options::default()), Err(NestError::NotFinite(1)), "{bad}");
        }
    }

    #[test]
    fn bad_regions_are_refused() {
        let parts = vec![rect(0.0, 0.0, 5.0, 5.0)];
        for r in [region(0.0, 10.0), region(10.0, -5.0), region(f64::NAN, 10.0), region(f64::INFINITY, 10.0), Rect::new(5.0, 5.0, 1.0, 1.0)] {
            assert_eq!(nest(&parts, r, &Options::default()), Err(NestError::Region), "{r:?}");
        }
        let margin_eats_it = Options { margin: 30.0, ..Options::default() };
        assert_eq!(nest(&parts, region(50.0, 50.0), &margin_eats_it), Err(NestError::Region));
    }

    #[test]
    fn bad_settings_are_refused() {
        let parts = vec![rect(0.0, 0.0, 5.0, 5.0)];
        let d = Options::default();
        for o in [
            Options { gap: f64::NAN, ..d },
            Options { gap: -1.0, ..d },
            Options { gap: 1e9, ..d },
            Options { margin: -1.0, ..d },
            Options { margin: f64::INFINITY, ..d },
            Options { rotations: 0, ..d },
            Options { rotations: MAX_ROTATIONS + 1, ..d },
            Options { resolution: Some(0.0), ..d },
            Options { resolution: Some(f64::NAN), ..d },
            Options { resolution: Some(-2.0), ..d },
        ] {
            assert!(matches!(nest(&parts, region(50.0, 50.0), &o), Err(NestError::Option(_))), "{o:?}");
        }
    }

    #[test]
    fn too_many_parts_or_points_are_refused() {
        let parts = vec![rect(0.0, 0.0, 1.0, 1.0); MAX_PARTS + 1];
        assert_eq!(nest(&parts, region(500.0, 500.0), &Options::default()), Err(NestError::TooManyParts));
        let many = Part { outlines: vec![Polyline::new(vec![Point::ORIGIN; MAX_POINTS / 2 + 1], true); 2] };
        assert_eq!(nest(&[many], region(50.0, 50.0), &Options::default()), Err(NestError::TooComplex));
    }

    #[test]
    fn odd_parts_are_reported_not_placed_and_nothing_panics() {
        let empty = Part::default();
        let no_points = Part { outlines: vec![Polyline::new(Vec::new(), true)] };
        let dot = Part { outlines: vec![Polyline::new(vec![Point::new(3.0, 3.0)], false)] };
        let huge = rect(0.0, 0.0, 1e12, 1e12);
        let two_points_closed = Part { outlines: vec![Polyline::new(vec![Point::new(0.0, 0.0), Point::new(4.0, 4.0)], true)] };
        let parts = vec![empty, no_points, dot, huge, two_points_closed];
        let result = nest(&parts, region(50.0, 50.0), &Options::default()).unwrap();
        assert!(result.unplaced.contains(&0) && result.unplaced.contains(&1) && result.unplaced.contains(&3), "{:?}", result.unplaced);
        assert!(result.placed.iter().any(|p| p.part == 2), "a single point is a part of one cell");
        assert_eq!(nest(&[], region(50.0, 50.0), &Options::default()).unwrap().placed.len(), 0);
    }

    #[test]
    fn huge_regions_and_gaps_scale_the_grid_instead_of_using_all_memory() {
        let parts = vec![rect(0.0, 0.0, 100.0, 100.0); 5];
        let result = nest(&parts, region(1e7, 1e7), &Options::default()).unwrap();
        assert!(result.cell >= 1.0, "a 1e7 mm region is not cut into 0.2 mm cells: {}", result.cell);
        let wide_gap = nest(&parts, region(500.0, 500.0), &Options { gap: 1000.0, ..Options::default() }).unwrap();
        assert!(wide_gap.cell >= 1000.0 / MAX_GAP_CELLS - 1e-9);
        let thin = nest(&parts, region(1e6, 0.5), &Options::default()).unwrap();
        assert!(thin.placed.is_empty());
        let tiny_cells = nest(&parts, region(500.0, 500.0), &Options { resolution: Some(1e-9), ..Options::default() }).unwrap();
        assert!(tiny_cells.cell >= 0.02, "{}", tiny_cells.cell);
    }
}
