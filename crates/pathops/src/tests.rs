use laserfoundry_geom::{Point, Polyline};

use super::*;

fn square(x: f64, y: f64, s: f64) -> Polyline {
    Polyline::new(vec![Point::new(x, y), Point::new(x + s, y), Point::new(x + s, y + s), Point::new(x, y + s)], true)
}

/// Net area of a set of contours (holes have opposite orientation, so they subtract).
fn area(lines: &[Polyline]) -> f64 {
    let signed: f64 = lines
        .iter()
        .map(|l| {
            let n = l.points.len();
            (0..n).map(|i| (l.points[i].x * l.points[(i + 1) % n].y - l.points[(i + 1) % n].x * l.points[i].y) / 2.0).sum::<f64>()
        })
        .sum();
    signed.abs()
}

mod boolean_tests {
    use super::*;

    #[test]
    fn overlapping_squares_combine_as_expected() {
        let (a, b) = (vec![square(0.0, 0.0, 4.0)], vec![square(2.0, 2.0, 4.0)]);
        assert!((area(&boolean(&a, &b, BoolOp::Union).unwrap()) - 28.0).abs() < 1e-6);
        assert!((area(&boolean(&a, &b, BoolOp::Intersect).unwrap()) - 4.0).abs() < 1e-6);
        assert!((area(&boolean(&a, &b, BoolOp::Difference).unwrap()) - 12.0).abs() < 1e-6);
        assert!((area(&boolean(&a, &b, BoolOp::Xor).unwrap()) - 24.0).abs() < 1e-6);
    }

    #[test]
    fn subtracting_an_inner_square_leaves_a_hole() {
        let r = boolean(&[square(0.0, 0.0, 10.0)], &[square(3.0, 3.0, 4.0)], BoolOp::Difference).unwrap();
        assert_eq!(r.len(), 2, "outer contour and hole");
        assert!((area(&r) - 84.0).abs() < 1e-6);
    }

    #[test]
    fn disjoint_shapes_stay_separate_and_empty_inputs_are_fine() {
        let r = boolean(&[square(0.0, 0.0, 2.0)], &[square(10.0, 10.0, 2.0)], BoolOp::Union).unwrap();
        assert_eq!(r.len(), 2);
        assert!(boolean(&[square(0.0, 0.0, 2.0)], &[square(10.0, 10.0, 2.0)], BoolOp::Intersect).unwrap().is_empty());
        assert!(boolean(&[], &[], BoolOp::Union).unwrap().is_empty());
    }

    #[test]
    fn a_self_intersecting_bowtie_is_resolved_under_even_odd() {
        let bowtie = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(4.0, 4.0), Point::new(4.0, 0.0), Point::new(0.0, 4.0)], true);
        let r = normalize(&[bowtie]).unwrap();
        assert_eq!(r.len(), 2, "two triangles");
        let total: f64 = r.iter().map(|l| area(std::slice::from_ref(l))).sum();
        assert!((total - 8.0).abs() < 1e-6, "two triangles of area 4: {total}");
    }

    #[test]
    fn open_and_degenerate_polylines_are_ignored_by_booleans() {
        let line = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(5.0, 5.0)], false);
        let tiny = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)], true);
        assert!(boolean(&[line, tiny], &[square(0.0, 0.0, 3.0)], BoolOp::Union).unwrap().len() == 1);
    }
}

mod offset_tests {
    use super::*;

    #[test]
    fn mitred_growth_and_shrink_of_a_square() {
        let sq = [square(0.0, 0.0, 4.0)];
        assert!((area(&offset(&sq, 1.0, Join::Miter).unwrap()) - 36.0).abs() < 1e-6);
        assert!((area(&offset(&sq, -1.0, Join::Miter).unwrap()) - 4.0).abs() < 1e-6);
    }

    #[test]
    fn round_joins_cut_the_corners_and_bevels_lie_between() {
        let sq = [square(0.0, 0.0, 10.0)];
        let (miter, round, bevel) = (
            area(&offset(&sq, 2.0, Join::Miter).unwrap()),
            area(&offset(&sq, 2.0, Join::Round).unwrap()),
            area(&offset(&sq, 2.0, Join::Bevel).unwrap()),
        );
        let exact_round = 100.0 + 4.0 * 10.0 * 2.0 + std::f64::consts::PI * 4.0;
        assert!((round - exact_round).abs() < 0.5, "{round} vs {exact_round}");
        assert!(miter > bevel && bevel > 100.0 + 80.0 && round < miter, "{miter} {round} {bevel}");
    }

    #[test]
    fn shrinking_past_collapse_gives_nothing_and_zero_is_a_no_op() {
        let sq = [square(0.0, 0.0, 4.0)];
        assert!(offset(&sq, -3.0, Join::Miter).unwrap().is_empty());
        assert!((area(&offset(&sq, 0.0, Join::Round).unwrap()) - 16.0).abs() < 1e-6);
    }

    #[test]
    fn holes_move_the_opposite_way() {
        let ring = [square(0.0, 0.0, 20.0), square(5.0, 5.0, 10.0)];
        // Growing by 1: outer 22², hole shrinks to 8².
        let grown = offset(&ring, 1.0, Join::Miter).unwrap();
        assert!((area(&grown) - (22.0 * 22.0 - 8.0 * 8.0)).abs() < 1e-6, "{}", area(&grown));
    }

    #[test]
    fn an_open_path_becomes_the_outline_around_it() {
        let line = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(20.0, 0.0)], false);
        let r = offset(std::slice::from_ref(&line), 1.0, Join::Round).unwrap();
        assert_eq!(r.len(), 1);
        let expected = 20.0 * 2.0 + std::f64::consts::PI;
        assert!((area(&r) - expected).abs() < 0.5, "{} vs {expected}", area(&r));
        assert_eq!(offset(std::slice::from_ref(&line), -1.0, Join::Round), Err(PathOpsError::OpenPathNeedsPositiveOffset));
        assert_eq!(offset(std::slice::from_ref(&line), 0.0, Join::Round), Err(PathOpsError::OpenPathNeedsPositiveOffset));
    }
}

mod hostile_tests {
    use super::*;

    #[test]
    fn non_finite_and_huge_inputs_are_errors() {
        let nan = Polyline::new(vec![Point::new(f64::NAN, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0)], true);
        assert_eq!(boolean(std::slice::from_ref(&nan), &[], BoolOp::Union), Err(PathOpsError::BadCoordinate));
        assert_eq!(offset(&[nan], 1.0, Join::Round), Err(PathOpsError::BadCoordinate));
        let far = square(1e9, 0.0, 1.0);
        assert_eq!(boolean(&[far], &[], BoolOp::Union), Err(PathOpsError::BadCoordinate));
    }

    #[test]
    fn bad_distances_are_errors() {
        for d in [f64::NAN, f64::INFINITY, 1e9, -1e9] {
            assert_eq!(offset(&[square(0.0, 0.0, 1.0)], d, Join::Round), Err(PathOpsError::BadDistance), "{d}");
        }
    }

    #[test]
    fn oversized_inputs_are_refused() {
        let big = Polyline::new((0..(MAX_POINTS + 10)).map(|i| Point::new((i % 1000) as f64, (i / 1000) as f64)).collect(), true);
        assert_eq!(boolean(&[big], &[], BoolOp::Union), Err(PathOpsError::TooLarge));
    }

    #[test]
    fn many_tiny_and_duplicate_shapes_do_not_panic() {
        let shapes: Vec<Polyline> = (0..200).map(|i| square(f64::from(i % 7) * 0.001, 0.0, 0.001)).collect();
        let _ = boolean(&shapes, &shapes, BoolOp::Xor);
        let _ = offset(&shapes, 0.0005, Join::Round);
        let zero_area = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(5.0, 0.0), Point::new(10.0, 0.0)], true);
        let _ = offset(std::slice::from_ref(&zero_area), 1.0, Join::Miter);
        let _ = boolean(&[zero_area], &[square(0.0, 0.0, 3.0)], BoolOp::Union);
    }
}

mod join_tests {
    use super::*;

    fn seg(a: (f64, f64), b: (f64, f64)) -> Polyline {
        Polyline::new(vec![Point::new(a.0, a.1), Point::new(b.0, b.1)], false)
    }

    #[test]
    fn four_loose_lines_become_one_closed_square_whatever_their_order_or_direction() {
        let lines = [seg((0.0, 0.0), (10.0, 0.0)), seg((10.0, 10.0), (10.0, 0.0)), seg((10.0, 10.0), (0.0, 10.0)), seg((0.0, 0.0), (0.0, 10.0))];
        let r = join(&lines, 0.01).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].closed && r[0].points.len() == 4, "{:?}", r[0]);
        assert!((area(&r) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn gaps_within_tolerance_are_bridged_and_larger_ones_are_not() {
        let lines = [seg((0.0, 0.0), (10.0, 0.0)), seg((10.04, 0.0), (20.0, 0.0))];
        assert_eq!(join(&lines, 0.05).unwrap().len(), 1);
        assert_eq!(join(&lines, 0.01).unwrap().len(), 2);
    }

    #[test]
    fn a_chain_that_does_not_close_stays_open_and_closed_shapes_are_untouched() {
        let lines = [seg((0.0, 0.0), (5.0, 0.0)), seg((5.0, 0.0), (5.0, 5.0)), square(100.0, 100.0, 3.0)];
        let r = join(&lines, 0.01).unwrap();
        assert_eq!(r.len(), 2);
        let open: Vec<_> = r.iter().filter(|l| !l.closed).collect();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].points.len(), 3);
        assert_eq!(r.iter().filter(|l| l.closed).count(), 1);
    }

    #[test]
    fn a_junction_does_not_loop_forever_or_lose_segments() {
        // A "T": three lines meeting at one point. All segments must be used exactly once.
        let lines = [seg((0.0, 0.0), (5.0, 0.0)), seg((10.0, 0.0), (5.0, 0.0)), seg((5.0, 0.0), (5.0, 5.0))];
        let r = join(&lines, 0.01).unwrap();
        let total: usize = r.iter().map(|l| l.points.len() - 1).sum();
        assert_eq!(total, 3, "{r:?}");
    }

    #[test]
    fn many_segments_of_a_circle_join_into_one_loop_quickly() {
        let n = 5000;
        let pts: Vec<Point> = (0..n)
            .map(|i| {
                Point::new(50.0 * (i as f64 / n as f64 * std::f64::consts::TAU).cos(), 50.0 * (i as f64 / n as f64 * std::f64::consts::TAU).sin())
            })
            .collect();
        let mut lines: Vec<Polyline> = (0..n).map(|i| Polyline::new(vec![pts[i], pts[(i + 1) % n]], false)).collect();
        lines.reverse();
        let r = join(&lines, 1e-6).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].closed && r[0].points.len() == n);
    }

    #[test]
    fn hostile_inputs_are_errors_not_panics() {
        assert_eq!(join(&[seg((0.0, 0.0), (1.0, 1.0))], f64::NAN), Err(PathOpsError::BadDistance));
        assert_eq!(join(&[seg((0.0, 0.0), (1.0, 1.0))], -1.0), Err(PathOpsError::BadDistance));
        assert_eq!(join(&[seg((f64::NAN, 0.0), (1.0, 1.0))], 0.01), Err(PathOpsError::BadCoordinate));
        assert!(join(&[], 0.01).unwrap().is_empty());
        let single = Polyline::new(vec![Point::new(0.0, 0.0)], false);
        assert!(join(&[single], 0.01).unwrap().is_empty(), "one-point polylines carry no geometry");
        assert!(join(&[seg((0.0, 0.0), (1.0, 1.0))], 0.0).is_ok(), "zero tolerance joins only exact matches");
    }
}
