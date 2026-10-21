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
            let options = Options {
                gap: [0.0, 1.0, 2.5, 4.0][round % 4],
                rotations: [1, 4, 8][round % 3],
                margin: [0.0, 3.0][round % 2],
                resolution: None,
                use_holes: false,
            };
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

mod obstacle_tests {
    use super::*;

    /// Every placed part keeps at least `gap` from every obstacle that is not itself outside the picture.
    fn clear_of(parts: &[Part], result: &Nesting, obstacles: &[Part], gap: f64) {
        for p in &result.placed {
            let lines = moved(&parts[p.part], p);
            for (k, o) in obstacles.iter().enumerate() {
                let d = distance(&lines, &o.outlines);
                assert!(d > 0.0, "part {} touches or overlaps obstacle {k}", p.part);
                assert!(d >= gap - 1e-6, "part {} is {d} mm from obstacle {k}, less than the gap {gap}", p.part);
            }
        }
    }

    #[test]
    fn parts_keep_the_gap_from_shapes_that_stay() {
        let parts: Vec<Part> = (0..6).map(|_| rect(0.0, 0.0, 12.0, 12.0)).collect();
        let obstacles = vec![rect(10.0, 10.0, 30.0, 30.0), rect(60.0, 0.0, 5.0, 50.0)];
        let options = Options { gap: 3.0, ..Options::default() };
        let result = nest_around(&parts, &obstacles, region(100.0, 60.0), &options).unwrap();
        assert!(result.placed.len() >= 5, "{} placed", result.placed.len());
        check(&parts, &result, region(100.0, 60.0), &options);
        clear_of(&parts, &result, &obstacles, 3.0);
    }

    #[test]
    fn a_region_that_is_blocked_places_nothing_and_reports_every_part() {
        let parts: Vec<Part> = (0..3).map(|_| rect(0.0, 0.0, 10.0, 10.0)).collect();
        let result = nest_around(&parts, &[rect(-5.0, -5.0, 100.0, 100.0)], region(50.0, 50.0), &Options::default()).unwrap();
        assert_eq!((result.placed.len(), result.unplaced.clone()), (0, vec![0, 1, 2]));
    }

    #[test]
    fn a_part_goes_into_the_one_gap_that_is_big_enough() {
        // Two walls leave a 16 mm slot; the 14 mm part with a 1 mm gap fits only there.
        let walls = vec![rect(0.0, 20.0, 30.0, 4.0), rect(46.0, 0.0, 4.0, 4.0), rect(0.0, 0.0, 30.0, 4.0), rect(30.0, 0.0, 4.0, 24.0)];
        let parts = vec![rect(0.0, 0.0, 14.0, 14.0)];
        let options = Options { gap: 1.0, rotations: 1, ..Options::default() };
        let result = nest_around(&parts, &walls, Rect::new(0.0, 0.0, 50.0, 24.0), &options).unwrap();
        assert_eq!(result.placed.len(), 1);
        let b = box_of(&moved(&parts[0], &result.placed[0]));
        assert!(b.x0 > 34.0 && b.x1 < 50.0 && b.y1 <= 24.0, "went to the open side: {b:?}");
        clear_of(&parts, &result, &walls, 1.0);
        let no_room =
            nest_around(&parts, &walls, Rect::new(0.0, 0.0, 50.0, 24.0), &Options { gap: 4.0, rotations: 1, ..Options::default() }).unwrap();
        assert_eq!(no_room.placed.len(), 0, "with a 4 mm gap the slot is too small");
    }

    #[test]
    fn obstacles_outside_or_with_bad_numbers_change_nothing() {
        let parts: Vec<Part> = (0..5).map(|_| rect(0.0, 0.0, 10.0, 10.0)).collect();
        let options = Options::default();
        let plain = nest(&parts, region(60.0, 40.0), &options).unwrap();
        let bad = Part { outlines: vec![Polyline::new(vec![Point::new(f64::NAN, 1.0), Point::new(5.0, 5.0), Point::new(9.0, 1.0)], true)] };
        let infinite = line((f64::INFINITY, 0.0), (0.0, 0.0));
        let elsewhere =
            vec![rect(1e9, 1e9, 10.0, 10.0), rect(-1e12, -1e12, 5.0, 5.0), line((-1e15, 100.0), (1e15, 100.0)), bad, infinite, Part::default()];
        let result = nest_around(&parts, &elsewhere, region(60.0, 40.0), &options).unwrap();
        assert_eq!(result, plain);
    }

    #[test]
    fn a_huge_shape_that_crosses_the_region_blocks_only_where_it_crosses() {
        // A 10 mm wide band, a billion mm long, runs across the middle of a 100 x 100 region.
        let band = rect(-5e8, 45.0, 1e9, 10.0);
        let parts: Vec<Part> = (0..4).map(|_| rect(0.0, 0.0, 20.0, 20.0)).collect();
        let options = Options { gap: 2.0, rotations: 1, ..Options::default() };
        let result = nest_around(&parts, std::slice::from_ref(&band), region(100.0, 100.0), &options).unwrap();
        assert_eq!(result.placed.len(), 4, "{:?}", result.unplaced);
        clear_of(&parts, &result, &[band], 2.0);
    }

    #[test]
    fn an_open_line_blocks_only_its_own_path() {
        let parts = vec![rect(0.0, 0.0, 10.0, 10.0)];
        let wall = line((0.0, 25.0), (40.0, 25.0));
        let options = Options { gap: 2.0, rotations: 1, ..Options::default() };
        // The region is 40 x 40: the part must sit above or below the line, never across it.
        let result = nest_around(&parts, std::slice::from_ref(&wall), region(40.0, 40.0), &options).unwrap();
        assert_eq!(result.placed.len(), 1);
        clear_of(&parts, &result, &[wall], 2.0);
    }

    #[test]
    fn random_obstacles_are_never_touched() {
        let mut seed = 7_u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f64) / f64::from(1u32 << 31)
        };
        for round in 0..6 {
            // Some obstacles hang over the edge of the region, some are triangles, some are open lines.
            let obstacles: Vec<Part> = (0..8)
                .map(|k| {
                    let (x, y) = (-20.0 + next() * 160.0, -20.0 + next() * 130.0);
                    match k % 3 {
                        0 => rect(x, y, 5.0 + next() * 25.0, 5.0 + next() * 25.0),
                        1 => Part {
                            outlines: vec![Polyline::new(
                                vec![Point::new(x, y), Point::new(x + 20.0 * next(), y + 5.0), Point::new(x + 8.0, y + 25.0 * next())],
                                true,
                            )],
                        },
                        _ => line((x, y), (x + 40.0 * next() - 20.0, y + 40.0 * next() - 20.0)),
                    }
                })
                .collect();
            let parts: Vec<Part> = (0..10).map(|_| rect(0.0, 0.0, 6.0 + next() * 10.0, 6.0 + next() * 10.0)).collect();
            let options = Options {
                gap: [0.0, 1.5, 3.0][round % 3],
                rotations: [1, 4][round % 2],
                margin: [0.0, 2.0][round % 2],
                resolution: None,
                use_holes: false,
            };
            let result = nest_around(&parts, &obstacles, region(120.0, 90.0), &options).unwrap();
            check(&parts, &result, region(120.0, 90.0), &options);
            // Only obstacles inside the region matter to a part, but clearance from all of them holds a fortiori.
            clear_of(&parts, &result, &obstacles, options.gap);
        }
    }

    #[test]
    fn thousands_of_obstacles_are_handled_quickly_and_too_many_are_refused() {
        let obstacles: Vec<Part> = (0..2000).map(|k| rect(f64::from(k % 100) * 3.0, f64::from(k / 100) * 3.0 + 200.0, 1.0, 1.0)).collect();
        let parts: Vec<Part> = (0..10).map(|_| rect(0.0, 0.0, 15.0, 15.0)).collect();
        let start = Instant::now();
        let result = nest_around(&parts, &obstacles, region(300.0, 200.0), &Options::default()).unwrap();
        assert_eq!(result.placed.len(), 10, "obstacles below the region do not matter");
        assert!(start.elapsed().as_secs() < 20, "took {:?}", start.elapsed());
        let too_many = vec![rect(0.0, 0.0, 1.0, 1.0); MAX_OBSTACLES + 1];
        assert_eq!(nest_around(&parts, &too_many, region(300.0, 200.0), &Options::default()), Err(NestError::TooManyObstacles));
        let heavy = Part { outlines: vec![Polyline::new(vec![Point::ORIGIN; MAX_POINTS / 2 + 1], true); 2] };
        assert_eq!(nest_around(&parts, &[heavy], region(50.0, 50.0), &Options::default()), Err(NestError::TooComplex), "obstacle points count too");
    }

    #[test]
    fn clipping_keeps_only_the_part_of_a_segment_inside_the_box() {
        assert_eq!(clip((-10.0, 5.0), (30.0, 5.0), 0.0, 0.0, 20.0, 10.0), Some(((0.0, 5.0), (20.0, 5.0))));
        assert_eq!(clip((5.0, 5.0), (6.0, 6.0), 0.0, 0.0, 20.0, 10.0), Some(((5.0, 5.0), (6.0, 6.0))), "inside stays whole");
        assert_eq!(clip((-10.0, -10.0), (-1.0, 50.0), 0.0, 0.0, 20.0, 10.0), None, "wholly outside");
        assert_eq!(clip((25.0, 0.0), (25.0, 10.0), 0.0, 0.0, 20.0, 10.0), None, "parallel and outside");
        assert!(clip((-1e15, -1e15), (1e15, 1e15), 0.0, 0.0, 20.0, 10.0).is_some_and(|(a, b)| a.0 >= -1e-6 && b.0 <= 20.0 + 1e-6));
        assert_eq!(clip((3.0, 3.0), (3.0, 3.0), 0.0, 0.0, 20.0, 10.0), Some(((3.0, 3.0), (3.0, 3.0))), "a point inside");
    }
}

mod hole_tests {
    use super::*;

    /// A square plate with a square hole in it, as one part: the plate is `size` mm across and `wall` mm thick all round.
    fn ring(x: f64, y: f64, size: f64, wall: f64) -> Part {
        let mut part = rect(x, y, size, size);
        part.outlines.extend(rect(x + wall, y + wall, size - 2.0 * wall, size - 2.0 * wall).outlines);
        part
    }

    /// Material by the even-odd rule over all of a part's closed outlines (a hole is not material).
    fn material(p: Point, lines: &[Polyline]) -> bool {
        lines.iter().filter(|l| inside(p, l)).count() % 2 == 1
    }

    /// The least distance between the edges of two sets of outlines, ignoring which lies inside which.
    fn edge_distance(a: &[Polyline], b: &[Polyline]) -> f64 {
        let mut least = f64::INFINITY;
        for (a0, a1) in a.iter().flat_map(edges) {
            for (b0, b1) in b.iter().flat_map(edges) {
                if segments_cross(a0, a1, b0, b1) {
                    return 0.0;
                }
                least =
                    least.min(point_segment(a0, b0, b1)).min(point_segment(a1, b0, b1)).min(point_segment(b0, a0, a1)).min(point_segment(b1, a0, a1));
            }
        }
        least
    }

    /// Nothing placed touches or overlaps anything else, with holes allowed: edges keep the gap, and no point of one part is in the
    /// other's material.
    fn check_with_holes(parts: &[Part], result: &Nesting, gap: f64) {
        let placed: Vec<Vec<Polyline>> = result.placed.iter().map(|p| moved(&parts[p.part], p)).collect();
        for i in 0..placed.len() {
            for j in (i + 1)..placed.len() {
                let d = edge_distance(&placed[i], &placed[j]);
                assert!(d > 0.0 && d >= gap - 1e-6, "parts {i} and {j} are {d} mm apart (gap {gap})");
                for (from, to) in [(i, j), (j, i)] {
                    let stray = placed[from].iter().flat_map(|l| l.points.iter()).find(|p| material(**p, &placed[to]));
                    assert!(stray.is_none(), "part {from} has a point in the material of part {to}: {stray:?}");
                }
            }
        }
    }

    #[test]
    fn a_small_part_goes_inside_the_hole_of_a_big_one_only_when_asked() {
        // The region is barely bigger than the plate: the 20 mm square can only go in its 40 mm hole.
        let parts = vec![ring(0.0, 0.0, 60.0, 10.0), rect(0.0, 0.0, 20.0, 20.0)];
        let options = Options { gap: 2.0, rotations: 1, use_holes: true, ..Options::default() };
        let result = nest(&parts, region(64.0, 64.0), &options).unwrap();
        assert_eq!(result.placed.len(), 2, "{:?}", result.unplaced);
        check_with_holes(&parts, &result, 2.0);
        let plate = result.placed.iter().find(|p| p.part == 0).unwrap();
        let square = result.placed.iter().find(|p| p.part == 1).unwrap();
        let (hole, sq) = (box_of(&moved(&parts[0], plate)[1..]), box_of(&moved(&parts[1], square)));
        assert!(
            sq.x0 >= hole.x0 + 2.0 - 1e-6 && sq.y0 >= hole.y0 + 2.0 - 1e-6 && sq.x1 <= hole.x1 - 2.0 + 1e-6 && sq.y1 <= hole.y1 - 2.0 + 1e-6,
            "{sq:?} in {hole:?}"
        );

        let off = Options { use_holes: false, ..options };
        let solid = nest(&parts, region(64.0, 64.0), &off).unwrap();
        assert_eq!((solid.placed.len(), solid.unplaced.clone()), (1, vec![1]), "by default a hole is solid");
    }

    #[test]
    fn an_island_inside_a_hole_is_solid_again() {
        // Plate 90 mm, hole 70 mm, island 30 mm in the middle of the hole (all one part).
        let mut part = ring(0.0, 0.0, 90.0, 10.0);
        part.outlines.extend(rect(30.0, 30.0, 30.0, 30.0).outlines);
        // The strip of hole beside the island is 20 mm wide, so 12 mm squares with 2 mm gaps fit it comfortably.
        let parts = vec![part, rect(0.0, 0.0, 12.0, 12.0), rect(0.0, 0.0, 12.0, 12.0), rect(0.0, 0.0, 12.0, 12.0)];
        let options = Options { gap: 2.0, rotations: 1, use_holes: true, ..Options::default() };
        let result = nest(&parts, region(94.0, 94.0), &options).unwrap();
        assert_eq!(result.placed.len(), 4, "{:?}", result.unplaced);
        check_with_holes(&parts, &result, 2.0);
        let plate = result.placed.iter().find(|p| p.part == 0).unwrap();
        let island = box_of(&moved(&parts[0], plate)[2..]);
        for p in result.placed.iter().filter(|p| p.part != 0) {
            let b = box_of(&moved(&parts[p.part], p));
            let apart =
                b.x1 <= island.x0 - 2.0 + 1e-6 || b.x0 >= island.x1 + 2.0 - 1e-6 || b.y1 <= island.y0 - 2.0 + 1e-6 || b.y0 >= island.y1 + 2.0 - 1e-6;
            assert!(apart, "{b:?} is on the island {island:?}");
        }
    }

    #[test]
    fn a_part_is_not_put_inside_an_island_in_a_hole() {
        // The strip of hole round the island is 20 mm wide, too narrow for a 20 mm square and its gaps; the island itself is 30 mm
        // across, so the square would fit there if the island were mistaken for room.
        let mut part = ring(0.0, 0.0, 90.0, 10.0);
        part.outlines.extend(rect(30.0, 30.0, 30.0, 30.0).outlines);
        let parts = vec![part, rect(0.0, 0.0, 20.0, 20.0)];
        let options = Options { gap: 2.0, rotations: 1, use_holes: true, ..Options::default() };
        let result = nest(&parts, region(94.0, 94.0), &options).unwrap();
        assert_eq!((result.placed.len(), result.unplaced.clone()), (1, vec![1]), "nothing may go on the island");
    }

    #[test]
    fn outlines_that_overlap_or_cross_are_not_holes() {
        let two = Part { outlines: rect(0.0, 0.0, 30.0, 30.0).outlines.into_iter().chain(rect(15.0, 15.0, 30.0, 30.0).outlines).collect() };
        assert_eq!(hole_depths(&two), Some(vec![0, 0]), "the overlap of two plates is not a hole");
        // A plate with a hole whose edge touches the plate's edge is not a hole either.
        let touching = Part { outlines: rect(0.0, 0.0, 30.0, 30.0).outlines.into_iter().chain(rect(0.0, 5.0, 10.0, 10.0).outlines).collect() };
        assert_eq!(hole_depths(&touching), Some(vec![0, 0]));
        let same = Part { outlines: rect(0.0, 0.0, 30.0, 30.0).outlines.into_iter().chain(rect(0.0, 0.0, 30.0, 30.0).outlines).collect() };
        assert_eq!(hole_depths(&same), Some(vec![0, 0]), "a doubled outline is not a hole");
        // A crescent: the small outline's corners are inside the big one but its edge leaves it.
        let notched = Polyline::new(
            vec![
                Point::new(0.0, 0.0),
                Point::new(40.0, 0.0),
                Point::new(40.0, 40.0),
                Point::new(25.0, 40.0),
                Point::new(25.0, 10.0),
                Point::new(15.0, 10.0),
                Point::new(15.0, 40.0),
                Point::new(0.0, 40.0),
            ],
            true,
        );
        let across = Polyline::new(vec![Point::new(5.0, 20.0), Point::new(35.0, 20.0), Point::new(35.0, 25.0), Point::new(5.0, 25.0)], true);
        assert_eq!(hole_depths(&Part { outlines: vec![notched, across] }), Some(vec![0, 0]), "an outline that crosses the other's edge is no hole");
    }

    #[test]
    fn depths_count_the_outlines_that_enclose_each_one() {
        let mut part = ring(0.0, 0.0, 90.0, 10.0);
        part.outlines.extend(rect(30.0, 30.0, 30.0, 30.0).outlines);
        part.outlines.push(Polyline::new(vec![Point::new(-5.0, -5.0), Point::new(200.0, 200.0)], false));
        assert_eq!(hole_depths(&part), Some(vec![0, 1, 2, 0]), "plate, hole, island, and an open line that is ignored");
        assert_eq!(hole_depths(&rect(0.0, 0.0, 10.0, 10.0)), None, "one outline has nothing to examine");
        let many = Part { outlines: (0..=MAX_HOLE_OUTLINES).map(|k| rect(f64::from(k as u32) * 3.0, 0.0, 2.0, 2.0).outlines.remove(0)).collect() };
        assert_eq!(hole_depths(&many), None, "too many outlines are treated as solid");
        let dense = Part { outlines: vec![Polyline::new(vec![Point::ORIGIN; MAX_HOLE_POINTS], true), rect(0.0, 0.0, 1.0, 1.0).outlines.remove(0)] };
        assert_eq!(hole_depths(&dense), None, "too many points are treated as solid");
    }

    #[test]
    fn a_rim_thinner_than_a_cell_is_never_built_on() {
        // A 0.4 mm rim round a 49.2 mm hole, with 1 mm cells: the rim's cells must stay taken.
        let parts = vec![ring(0.0, 0.0, 50.0, 0.4), rect(0.0, 0.0, 4.0, 4.0), rect(0.0, 0.0, 4.0, 4.0), rect(0.0, 0.0, 4.0, 4.0)];
        let options = Options { gap: 0.5, rotations: 1, use_holes: true, resolution: Some(1.0), ..Options::default() };
        let result = nest(&parts, region(54.0, 54.0), &options).unwrap();
        assert_eq!(result.placed.len(), 4, "{:?}", result.unplaced);
        check_with_holes(&parts, &result, 0.5);
    }

    #[test]
    fn random_plates_and_parts_with_holes_never_overlap_when_turned() {
        let mut seed = 31_u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f64) / f64::from(1u32 << 31)
        };
        for round in 0..8 {
            let mut parts: Vec<Part> = (0..3).map(|_| ring(0.0, 0.0, 40.0 + next() * 20.0, 4.0 + next() * 6.0)).collect();
            parts.extend((0..12).map(|_| rect(0.0, 0.0, 4.0 + next() * 10.0, 4.0 + next() * 10.0)));
            let options = Options { gap: [0.0, 1.0, 2.5][round % 3], rotations: [1, 4, 8][round % 3], use_holes: true, ..Options::default() };
            let result = nest(&parts, region(150.0, 100.0), &options).unwrap();
            assert!(result.placed.len() >= 8, "round {round}: {} placed", result.placed.len());
            check_with_holes(&parts, &result, options.gap);
        }
    }

    #[test]
    fn holes_make_room_that_solid_parts_do_not() {
        let parts: Vec<Part> = std::iter::once(ring(0.0, 0.0, 70.0, 12.0)).chain((0..6).map(|_| rect(0.0, 0.0, 16.0, 16.0))).collect();
        let on = Options { gap: 2.0, rotations: 1, use_holes: true, ..Options::default() };
        let off = Options { use_holes: false, ..on };
        let with = nest(&parts, region(74.0, 74.0), &on).unwrap();
        let without = nest(&parts, region(74.0, 74.0), &off).unwrap();
        assert!(with.placed.len() > without.placed.len(), "{} with holes, {} without", with.placed.len(), without.placed.len());
        check_with_holes(&parts, &with, 2.0);
    }
}

mod mask_tests {
    use super::*;

    fn mask_of(part: &Part, cell: f64) -> Mask {
        let b = bounds(part).unwrap();
        Mask::of(part, b.center(), 0.0, cell, 1000, 1000, None).unwrap()
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
        assert!(Mask::of(&big, Point::ORIGIN, 0.0, 0.1, 100, 100, None).is_none());
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
