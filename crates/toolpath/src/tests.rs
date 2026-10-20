use std::sync::Arc;

use laserfoundry_doc::{Document, LayerMode, RasterImage, Shape, ShapeKind};
use laserfoundry_geom::{Affine, Point, Polyline, ellipse_path, rect_path};

use super::*;

fn rect_doc(w: f64, h: f64, mode: LayerMode) -> Document {
    let mut doc = Document::default();
    if let Some(l) = doc.layer_mut(0) {
        l.mode = mode;
    }
    doc.add_shape(Shape::path("r", 0, rect_path(10.0, 10.0, w, h)));
    doc
}

mod line_tests {
    use super::*;

    #[test]
    fn rectangle_is_travel_then_four_cuts_back_to_start() {
        let job = plan(&rect_doc(20.0, 10.0, LayerMode::Line)).unwrap();
        assert_eq!(job.ops.len(), 1);
        let moves = &job.ops[0].moves;
        assert!(matches!(moves[0], Move::Travel(_)));
        assert_eq!(moves.len(), 1 + 4);
        assert_eq!(moves.last().map(Move::target), moves.first().map(Move::target));
    }

    #[test]
    fn inner_shapes_are_cut_before_outer() {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("outer", 0, rect_path(0.0, 0.0, 100.0, 100.0)));
        doc.add_shape(Shape::path("inner", 0, ellipse_path(50.0, 50.0, 10.0, 10.0)));
        let job = plan(&doc).unwrap();
        let first = job.ops[0].moves[0].target();
        // The circle (near the centre) is cut first even though the rectangle's corner is nearer the origin.
        assert!(first.distance(Point::new(50.0, 50.0)) < 15.0, "{first:?}");
    }

    #[test]
    fn disabled_layers_produce_no_output() {
        let mut doc = rect_doc(5.0, 5.0, LayerMode::Line);
        if let Some(l) = doc.layer_mut(0) {
            l.output = false;
        }
        assert!(plan(&doc).unwrap().ops.is_empty());
    }
}

mod kerf_tests {
    use super::*;

    fn bounds(job: &Job) -> laserfoundry_geom::Rect {
        laserfoundry_geom::bounds_of_points(job.ops.iter().flat_map(|o| o.moves.iter().map(Move::target))).unwrap()
    }

    #[test]
    fn kerf_grows_the_cut_path_around_a_part_and_shrinks_it_inside_a_hole() {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("part", 0, rect_path(0.0, 0.0, 40.0, 40.0)));
        doc.add_shape(Shape::path("hole", 0, rect_path(10.0, 10.0, 20.0, 20.0)));
        let plain = bounds(&plan(&doc).unwrap());
        assert_eq!((plain.x0, plain.x1), (0.0, 40.0));
        doc.layer_mut(0).unwrap().kerf_offset = 0.1;
        let job = plan(&doc).unwrap();
        let b = bounds(&job);
        assert!((b.x0 + 0.1).abs() < 1e-6 && (b.x1 - 40.1).abs() < 1e-6, "outer path moved out by 0.1: {b:?}");
        // The hole contour is cut first (inner-first) and sits 0.1 mm *inside* the hole.
        let first = job.ops[0].moves[0].target();
        assert!(first.x > 10.05 && first.x < 29.95, "{first:?}");
    }

    #[test]
    fn negative_kerf_shrinks_parts_and_open_paths_are_untouched() {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("part", 0, rect_path(0.0, 0.0, 40.0, 40.0)));
        doc.add_shape(Shape::path("line", 0, laserfoundry_geom::line_path(Point::new(100.0, 100.0), Point::new(120.0, 100.0))));
        doc.layer_mut(0).unwrap().kerf_offset = -0.2;
        let job = plan(&doc).unwrap();
        let xs: Vec<f64> = job.ops[0].moves.iter().map(|m| m.target().x).collect();
        assert!(xs.iter().any(|x| (x - 0.2).abs() < 1e-6) && xs.iter().any(|x| (x - 39.8).abs() < 1e-6));
        assert!(xs.iter().any(|x| (x - 100.0).abs() < 1e-9) && xs.iter().any(|x| (x - 120.0).abs() < 1e-9), "the open line is cut as drawn");
    }

    #[test]
    fn a_shape_that_collapses_under_kerf_is_dropped_and_bad_kerf_is_rejected() {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("tiny", 0, rect_path(0.0, 0.0, 0.1, 0.1)));
        doc.layer_mut(0).unwrap().kerf_offset = -1.0;
        assert!(plan(&doc).unwrap().ops.is_empty());
        for bad in [f64::NAN, 1e9] {
            doc.layer_mut(0).unwrap().kerf_offset = bad;
            assert!(matches!(plan(&doc), Err(PlanError::BadLayerSetting { .. })), "{bad}");
        }
    }

    #[test]
    fn fill_ignores_kerf() {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("part", 0, rect_path(0.0, 0.0, 10.0, 10.0)));
        let layer = doc.layer_mut(0).unwrap();
        layer.mode = LayerMode::Fill;
        layer.interval = 1.0;
        layer.kerf_offset = 0.5;
        let b = bounds(&plan(&doc).unwrap());
        assert_eq!((b.x0, b.x1), (0.0, 10.0));
    }
}

mod override_tests {
    use laserfoundry_doc::CutOverride;

    use super::*;

    fn two_squares() -> Document {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("plain", 0, rect_path(0.0, 0.0, 10.0, 10.0)));
        doc.add_shape(Shape::path("special", 0, rect_path(50.0, 0.0, 10.0, 10.0)));
        let layer = doc.layer_mut(0).unwrap();
        layer.speed = 1000.0;
        layer.max_power = 40.0;
        doc
    }

    #[test]
    fn an_overridden_shape_runs_at_its_own_speed_and_power_and_the_rest_do_not() {
        let mut doc = two_squares();
        doc.shapes[1].cut = Some(CutOverride { speed: Some(300.0), max_power: Some(90.0), ..Default::default() });
        let job = plan(&doc).unwrap();
        assert_eq!(job.ops.len(), 2);
        let (plain, special) = (&job.ops[0], &job.ops[1]);
        assert_eq!(plain.speed, 1000.0);
        assert!(plain.moves.iter().all(|m| !matches!(m, Move::Cut { power, .. } if *power != 40.0)));
        assert_eq!(special.speed, 300.0);
        assert!(special.moves.iter().filter(|m| matches!(m, Move::Cut { .. })).all(|m| matches!(m, Move::Cut { power, .. } if *power == 90.0)));
        assert!(special.moves[0].target().x >= 50.0, "the overridden square is the second op");
    }

    #[test]
    fn shapes_with_equal_overrides_share_an_op_and_empty_overrides_are_ignored() {
        let mut doc = two_squares();
        doc.shapes[0].cut = Some(CutOverride::default());
        doc.shapes[1].cut = Some(CutOverride { passes: Some(3), ..Default::default() });
        let job = plan(&doc).unwrap();
        assert_eq!(job.ops.len(), 2);
        assert_eq!((job.ops[0].passes, job.ops[1].passes), (1, 3));
        let mut doc = two_squares();
        let same = Some(CutOverride { speed: Some(500.0), ..Default::default() });
        doc.shapes[0].cut = same;
        doc.shapes[1].cut = same;
        assert_eq!(plan(&doc).unwrap().ops.len(), 1, "one op for both");
    }

    #[test]
    fn bad_override_values_are_rejected_like_bad_layer_values() {
        let mut doc = two_squares();
        doc.shapes[1].cut = Some(CutOverride { speed: Some(f64::NAN), ..Default::default() });
        assert!(matches!(plan(&doc), Err(PlanError::BadLayerSetting { .. })));
        doc.shapes[1].cut = Some(CutOverride { speed: Some(-5.0), ..Default::default() });
        assert!(matches!(plan(&doc), Err(PlanError::BadLayerSetting { .. })));
    }
}

mod tab_and_lead_tests {
    use super::*;

    fn square_doc(size: f64, at: f64) -> Document {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("part", 0, rect_path(at, at, size, size)));
        doc
    }

    /// Lengths of (powered, unpowered) cut moves in the first op, from each move's predecessor.
    fn lengths(job: &Job) -> (f64, f64) {
        let (mut on, mut off) = (0.0, 0.0);
        let mut at = None;
        for m in &job.ops[0].moves {
            if let (Some(a), Move::Cut { to, power }) = (at, m) {
                let d = Point::distance(a, *to);
                if *power > 0.0 { on += d } else { off += d }
            }
            at = Some(m.target());
        }
        (on, off)
    }

    #[test]
    fn tabs_leave_gaps_with_the_laser_off_and_keep_cutting_speed() {
        let mut doc = square_doc(40.0, 10.0);
        let layer = doc.layer_mut(0).unwrap();
        layer.tab_count = 4;
        layer.tab_width = 2.0;
        let job = plan(&doc).unwrap();
        let (on, off) = lengths(&job);
        assert!((off - 8.0).abs() < 1e-6 && (on - 152.0).abs() < 1e-6, "{on} {off}");
        let gaps = job.ops[0]
            .moves
            .windows(2)
            .filter(|w| matches!(w[1], Move::Cut { power, .. } if power == 0.0) && !matches!(w[0], Move::Cut { power, .. } if power == 0.0))
            .count();
        assert_eq!(gaps, 4, "four separate tabs");
    }

    #[test]
    fn holes_get_no_tabs_but_outer_outlines_do() {
        let mut doc = square_doc(60.0, 10.0);
        doc.add_shape(Shape::path("hole", 0, rect_path(30.0, 30.0, 10.0, 10.0)));
        let layer = doc.layer_mut(0).unwrap();
        layer.tab_count = 2;
        layer.tab_width = 3.0;
        let (_, off) = lengths(&plan(&doc).unwrap());
        assert!((off - 6.0).abs() < 1e-6, "two tabs on the outer outline only, got {off}");
    }

    #[test]
    fn tabs_that_do_not_fit_leave_the_outline_whole() {
        let mut doc = square_doc(4.0, 10.0);
        let layer = doc.layer_mut(0).unwrap();
        layer.tab_count = 4;
        layer.tab_width = 3.0;
        let (on, off) = lengths(&plan(&doc).unwrap());
        assert_eq!(off, 0.0);
        assert!((on - 16.0).abs() < 1e-6);
    }

    #[test]
    fn a_lead_in_approaches_from_the_scrap_side_and_a_hole_leads_inside_itself() {
        let mut doc = square_doc(60.0, 10.0);
        doc.add_shape(Shape::path("hole", 0, rect_path(30.0, 30.0, 10.0, 10.0)));
        let layer = doc.layer_mut(0).unwrap();
        layer.lead_in = 4.0;
        layer.lead_out = 2.0;
        let job = plan(&doc).unwrap();
        let moves = &job.ops[0].moves;
        // Inner-first: the hole is cut first.
        let (entry, start) = (moves[0].target(), moves[1].target());
        assert!((entry.distance(start) - 4.0).abs() < 1e-9, "{entry:?} -> {start:?}");
        assert!(entry.x > 30.0 && entry.x < 40.0 && entry.y > 30.0 && entry.y < 40.0, "the hole's lead-in is inside the hole: {entry:?}");
        assert!(matches!(moves[0], Move::Travel(_)) && matches!(moves[1], Move::Cut { power, .. } if power > 0.0), "the approach is a powered cut");
        // The outer outline's lead-in starts outside the 60 mm square.
        let outer = moves.iter().enumerate().skip(2).find(|(_, m)| matches!(m, Move::Travel(_))).unwrap().0;
        let entry = moves[outer].target();
        assert!(entry.x < 10.0 || entry.x > 70.0 || entry.y < 10.0 || entry.y > 70.0, "{entry:?}");
        // The lead-out leaves the outline again after it has closed.
        let last = moves.last().unwrap().target();
        let first_cut = moves[outer + 1].target();
        assert!((last.distance(first_cut) - 2.0).abs() < 1e-9, "{last:?} vs {first_cut:?}");
    }

    #[test]
    fn leads_are_shortened_to_stay_on_the_bed_for_absolute_placement_only() {
        let mut doc = square_doc(20.0, 0.0); // flush against the bed's top-left corner
        doc.layer_mut(0).unwrap().lead_in = 5.0;
        for m in &plan(&doc).unwrap().ops[0].moves {
            let p = m.target();
            assert!(p.x >= 0.0 && p.y >= 0.0, "{p:?} is off the bed");
        }
        doc.job_origin.start_from = laserfoundry_doc::StartFrom::CurrentPosition;
        let job = plan(&doc).unwrap();
        assert!(job.ops[0].moves.iter().any(|m| m.target().y < 0.0 || m.target().x < 0.0), "relative placement keeps the full lead");
    }

    #[test]
    fn open_paths_and_fill_are_untouched_and_bad_values_are_rejected() {
        let mut doc = Document::default();
        doc.add_shape(Shape::path("line", 0, laserfoundry_geom::line_path(Point::new(10.0, 10.0), Point::new(50.0, 10.0))));
        let layer = doc.layer_mut(0).unwrap();
        layer.lead_in = 3.0;
        layer.tab_count = 3;
        let job = plan(&doc).unwrap();
        assert_eq!(job.ops[0].moves.len(), 2, "travel + one cut, no lead, no tabs");
        for field in 0..3 {
            let mut bad = doc.clone();
            let layer = bad.layer_mut(0).unwrap();
            match field {
                0 => layer.tab_width = f64::NAN,
                1 => layer.lead_in = -1.0,
                _ => layer.lead_out = 1e9,
            }
            assert!(matches!(plan(&bad), Err(PlanError::BadLayerSetting { .. })), "field {field}");
        }
    }
}

mod fill_tests {
    use super::*;

    #[test]
    fn fill_covers_rectangle_with_expected_row_count() {
        let mut doc = rect_doc(10.0, 10.0, LayerMode::Fill);
        if let Some(l) = doc.layer_mut(0) {
            l.interval = 0.5;
        }
        let job = plan(&doc).unwrap();
        assert_eq!(job.ops[0].kind, OpKind::Fill);
        let cuts = job.ops[0].moves.iter().filter(|m| matches!(m, Move::Cut { .. })).count();
        assert_eq!(cuts, 20);
    }

    #[test]
    fn an_ellipse_fills_with_rows_shorter_near_its_poles() {
        let mut doc = Document::default();
        if let Some(l) = doc.layer_mut(0) {
            l.mode = LayerMode::Fill;
            l.interval = 1.0;
        }
        doc.add_shape(Shape::path("e", 0, ellipse_path(50.0, 50.0, 20.0, 10.0)));
        let job = plan(&doc).unwrap();
        let spans: Vec<f64> = job.ops[0].moves.chunks(2).map(|c| c[0].target().distance(c[1].target())).collect();
        assert_eq!(spans.len(), 20, "one row per mm over a 20 mm tall ellipse");
        assert!(spans[10] > 39.0 && spans[0] < 15.0, "wide in the middle, narrow at the pole: {spans:?}");
    }

    #[test]
    fn bidirectional_fill_alternates_direction() {
        let segs = fill_segments(&laserfoundry_geom::flatten(&rect_path(0.0, 0.0, 10.0, 10.0), 0.1), 1.0, 0.0, true);
        assert!(segs[0].0.x < segs[0].1.x);
        assert!(segs[1].0.x > segs[1].1.x);
    }

    #[test]
    fn holes_are_not_filled() {
        let mut lines = laserfoundry_geom::flatten(&rect_path(0.0, 0.0, 10.0, 10.0), 0.1);
        lines.extend(laserfoundry_geom::flatten(&rect_path(4.0, 0.0, 2.0, 10.0), 0.1));
        let segs = fill_segments(&lines, 1.0, 0.0, false);
        assert_eq!(segs.len(), 20);
    }

    #[test]
    fn hostile_interval_does_not_explode() {
        let lines = laserfoundry_geom::flatten(&rect_path(0.0, 0.0, 1e9, 1e9), 0.1);
        assert!(fill_segments(&lines, 1e-9, 0.0, false).is_empty());
        assert!(fill_segments(&lines, f64::NAN, 0.0, false).is_empty());
    }
}

mod image_tests {
    use super::*;

    fn image_doc(gray: Vec<u8>, w: u32, h: u32) -> Document {
        let mut doc = Document::default();
        if let Some(l) = doc.layer_mut(0) {
            l.interval = 1.0;
            l.min_power = 0.0;
            l.max_power = 100.0;
        }
        let image = Arc::new(RasterImage::new(w, h, gray).unwrap());
        doc.add_shape(Shape {
            id: 0,
            name: "img".into(),
            layer: 0,
            transform: Affine::IDENTITY,
            kind: ShapeKind::Image { image, width_mm: w as f64, height_mm: h as f64, adjust: Default::default() },
            cut: None,
            group: None,
            order: 0,
        });
        doc
    }

    #[test]
    fn white_image_burns_nothing() {
        let job = plan(&image_doc(vec![255; 16], 4, 4)).unwrap();
        assert!(job.ops.is_empty());
    }

    #[test]
    fn black_image_burns_full_power_with_overscan() {
        let job = plan(&image_doc(vec![0; 16], 4, 4)).unwrap();
        let moves = &job.ops[0].moves;
        let full: Vec<_> = moves.iter().filter(|m| matches!(m, Move::Cut { power, .. } if *power == 100.0)).collect();
        assert_eq!(full.len(), 4);
        // Overscan lead-in and lead-out rows exist at zero power.
        assert!(moves.iter().any(|m| matches!(m, Move::Cut { power, .. } if *power == 0.0)));
    }

    #[test]
    fn a_picture_flush_with_the_bed_edge_gets_a_shorter_lead_in_instead_of_leaving_the_bed() {
        let mut doc = image_doc(vec![0; 16], 4, 4);
        doc.machine.bed_width = 4.0;
        let job = plan(&doc).unwrap();
        for m in &job.ops[0].moves {
            let x = m.target().x;
            assert!((0.0..=4.0).contains(&x), "{x} is outside the bed");
        }
        // Relative placement cannot know the room, so the full lead-in is kept.
        doc.job_origin.start_from = laserfoundry_doc::StartFrom::UserOrigin;
        let job = plan(&doc).unwrap();
        assert!(job.ops[0].moves.iter().any(|m| m.target().x < 0.0), "lead-in extends past the picture");
    }

    #[test]
    fn a_negative_burns_the_picture_but_never_the_white_around_a_rotated_one() {
        let mut doc = image_doc(vec![255; 16], 4, 4); // an all-white 4x4 mm picture
        if let Some(ShapeKind::Image { adjust, .. }) = doc.shapes.get_mut(0).map(|s| &mut s.kind) {
            adjust.negative = true;
        }
        doc.shapes[0].transform = Affine::translate((50.0, 50.0)) * Affine::rotate(0.6) * Affine::IDENTITY;
        let job = plan(&doc).unwrap();
        let burned: Vec<_> = job.ops[0].moves.iter().filter(|m| matches!(m, Move::Cut { power, .. } if *power > 0.0)).collect();
        assert!(!burned.is_empty(), "the white picture, negated, is black");
        let shape = &doc.shapes[0];
        let inside = shape.world_polylines().into_iter().next().unwrap();
        // Every burning move ends within a pixel of the picture: nothing outside it burns.
        for m in burned {
            let p = m.target();
            assert!(inside.contains(p) || inside.distance_to(p) <= 1.5, "{p:?} burns outside the picture");
        }
        // And the un-negated white picture burns nothing at all.
        if let Some(ShapeKind::Image { adjust, .. }) = doc.shapes.get_mut(0).map(|s| &mut s.kind) {
            adjust.negative = false;
        }
        assert!(plan(&doc).unwrap().ops.is_empty());
    }

    #[test]
    fn brightness_lightens_a_gray_image_until_it_stops_burning() {
        let mut doc = image_doc(vec![100; 16], 4, 4);
        if let Some(l) = doc.layer_mut(0) {
            l.dither = laserfoundry_doc::Dither::Grayscale;
        }
        let power =
            |doc: &Document| -> f64 {
                plan(doc).unwrap().ops.first().map_or(0.0, |o| {
                    o.moves.iter().filter_map(|m| if let Move::Cut { power, .. } = m { Some(*power) } else { None }).fold(0.0, f64::max)
                })
            };
        let base = power(&doc);
        assert!(base > 50.0);
        if let Some(ShapeKind::Image { adjust, .. }) = doc.shapes.get_mut(0).map(|s| &mut s.kind) {
            adjust.brightness = 0.3;
        }
        assert!(power(&doc) < base - 20.0, "brighter picture burns less");
        if let Some(ShapeKind::Image { adjust, .. }) = doc.shapes.get_mut(0).map(|s| &mut s.kind) {
            adjust.brightness = 1.0;
        }
        assert_eq!(power(&doc), 0.0);
    }

    #[test]
    fn oversized_raster_is_refused() {
        let mut doc = image_doc(vec![0; 4], 2, 2);
        if let Some(l) = doc.layer_mut(0) {
            l.interval = 0.0001;
        }
        if let Some(s) = doc.shapes.first_mut() {
            s.transform = Affine::scale(1000.0);
        }
        assert!(matches!(plan(&doc), Err(PlanError::RasterTooLarge(_))));
    }

    #[test]
    fn floyd_steinberg_preserves_mean_darkness() {
        let gray = vec![128.0f32; 100 * 100];
        let out = dither(&gray, 100, 100, laserfoundry_doc::Dither::FloydSteinberg);
        let mean: f32 = out.iter().sum::<f32>() / out.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
    }

    #[test]
    fn every_dither_mode_keeps_the_overall_tone_and_the_extremes() {
        use laserfoundry_doc::Dither;
        for mode in Dither::ALL {
            if mode == Dither::Threshold {
                continue; // a threshold has no tone to keep, only the extremes below
            }
            for (level, expected) in [(64.0f32, 0.75f32), (128.0, 0.5), (192.0, 0.25)] {
                let out = dither(&vec![level; 120 * 120], 120, 120, mode);
                let mean: f32 = out.iter().sum::<f32>() / out.len() as f32;
                let tolerance = if mode == Dither::Atkinson { 0.25 } else { 0.06 }; // Atkinson drops a quarter of the error on purpose
                assert!((mean - expected).abs() < tolerance, "{mode:?} at {level}: {mean}");
            }
            let white = dither(&vec![255.0; 40 * 40], 40, 40, mode);
            let black = dither(&vec![0.0; 40 * 40], 40, 40, mode);
            assert!(white.iter().all(|v| *v == 0.0), "{mode:?}: white burns nothing");
            assert!(black.iter().all(|v| *v == 1.0), "{mode:?}: black burns everything");
        }
    }

    #[test]
    fn dithers_survive_odd_sizes_and_mismatched_buffers() {
        use laserfoundry_doc::Dither;
        for mode in Dither::ALL {
            assert!(dither(&[], 0, 0, mode).is_empty());
            assert_eq!(dither(&[10.0], 1, 1, mode).len(), 1);
            assert_eq!(dither(&[10.0; 5], 3, 3, mode).len(), 5, "{mode:?}: a short buffer must not index out of range");
            assert_eq!(dither(&[99.0; 7 * 3], 7, 3, mode).len(), 21);
        }
    }

    #[test]
    fn the_bayer_ordered_pattern_is_regular_and_a_halftone_forms_dots() {
        use laserfoundry_doc::Dither;
        let g = vec![128.0f32; 16 * 16];
        let out = dither(&g, 16, 16, Dither::Ordered);
        assert_eq!(out[..8], out[8..16], "the 8-pixel tile repeats");
        let half = dither(&vec![128.0; 48 * 48], 48, 48, Dither::Halftone);
        let rows_with_both = (0..48).filter(|y| half[y * 48..(y + 1) * 48].contains(&1.0) && half[y * 48..(y + 1) * 48].contains(&0.0)).count();
        assert!(rows_with_both > 40, "dots, not a solid fill: {rows_with_both}");
    }
}

mod validation_tests {
    use super::*;

    #[test]
    fn bad_speed_is_an_error_not_a_panic() {
        let mut doc = rect_doc(5.0, 5.0, LayerMode::Line);
        for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            if let Some(l) = doc.layer_mut(0) {
                l.speed = bad;
            }
            assert!(matches!(plan(&doc), Err(PlanError::BadLayerSetting { .. })));
        }
    }

    #[test]
    fn estimate_scales_with_passes() {
        let mut doc = rect_doc(60.0, 0.0001, LayerMode::Line);
        if let Some(l) = doc.layer_mut(0) {
            l.speed = 600.0;
            l.passes = 1;
        }
        let one = estimate(&plan(&doc).unwrap(), 6000.0);
        if let Some(l) = doc.layer_mut(0) {
            l.passes = 3;
        }
        let three = estimate(&plan(&doc).unwrap(), 6000.0);
        assert!((three.seconds / one.seconds - 3.0).abs() < 0.2);
    }
}

mod timeline_tests {
    use super::*;
    use crate::timeline::corner_speed;

    /// An operation that travels to the first point and then cuts through the rest at `speed` mm/min.
    fn polyline_op(points: &[(f64, f64)], speed: f64) -> Op {
        let mut moves = vec![Move::Travel(Point::new(points[0].0, points[0].1))];
        moves.extend(points[1..].iter().map(|&(x, y)| Move::Cut { to: Point::new(x, y), power: 50.0 }));
        Op { layer: 0, kind: OpKind::Line, speed, passes: 1, air_assist: false, z: 0.0, z_per_pass: 0.0, moves }
    }

    fn seconds(op: &Op, motion: Motion) -> f64 {
        move_times(op, Point::new(op.moves[0].target().x, op.moves[0].target().y), 6000.0, motion).iter().sum()
    }

    const CORNERING: Motion = Motion { acceleration: 500.0, junction_deviation: 0.01 };
    const REST_TO_REST: Motion = Motion { acceleration: 500.0, junction_deviation: 0.0 };

    #[test]
    fn a_straight_line_cut_in_many_pieces_takes_as_long_as_one_move() {
        // 100 mm at 50 mm/s with 500 mm/s²: 2 s of cruising plus 0.1 s of speeding up and slowing down.
        let pieces: Vec<(f64, f64)> = (0..=100).map(|i| (f64::from(i), 0.0)).collect();
        let one = polyline_op(&[(0.0, 0.0), (100.0, 0.0)], 3000.0);
        let many = polyline_op(&pieces, 3000.0);
        assert!((seconds(&one, CORNERING) - 2.1).abs() < 1e-9);
        assert!((seconds(&many, CORNERING) - 2.1).abs() < 1e-6, "{}", seconds(&many, CORNERING));
        // Timed move by move from rest, every piece would pay for its own run-up: the old estimate was far too slow.
        assert!(seconds(&many, REST_TO_REST) > 4.0, "{}", seconds(&many, REST_TO_REST));
    }

    #[test]
    fn corner_speed_follows_the_junction_deviation_formula() {
        let (east, north, west) = ((1.0, 0.0), (0.0, 1.0), (-1.0, 0.0));
        assert!(corner_speed(east, east, 500.0, 0.01).is_infinite(), "no turn, no limit");
        assert!(corner_speed(east, west, 500.0, 0.01).abs() < 1e-9, "turning straight back stops the head");
        // A right angle: sin(θ/2) = sqrt(1/2), R = δ·s/(1−s), v = sqrt(a·R).
        let s = 0.5_f64.sqrt();
        let expected = (500.0 * 0.01 * s / (1.0 - s)).sqrt();
        assert!((corner_speed(east, north, 500.0, 0.01) - expected).abs() < 1e-9);
        assert!((expected - 3.4745).abs() < 1e-3, "{expected}");
        // A gentler bend is taken faster, and a larger deviation allows more speed.
        let gentle = (0.9_f64.acos().cos(), 0.9_f64.acos().sin());
        assert!(corner_speed(east, gentle, 500.0, 0.01) > corner_speed(east, north, 500.0, 0.01));
        assert!(corner_speed(east, north, 500.0, 0.05) > corner_speed(east, north, 500.0, 0.01));
    }

    #[test]
    fn a_square_is_faster_with_cornering_but_not_faster_than_the_speed_limit() {
        let square = polyline_op(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0), (0.0, 0.0)], 3000.0);
        let (planned, from_rest) = (seconds(&square, CORNERING), seconds(&square, REST_TO_REST));
        let limit = 80.0 / 50.0; // 80 mm at 50 mm/s with nothing slowing it
        assert!(planned < from_rest, "{planned} vs {from_rest}");
        assert!(planned > limit, "{planned} vs {limit}");
        // The square has four right angles but the cut starts and ends at rest at one of them; a straight cut of the
        // same length is quicker than the square.
        let straight = polyline_op(&[(0.0, 0.0), (80.0, 0.0)], 3000.0);
        assert!(seconds(&straight, CORNERING) < planned);
    }

    #[test]
    fn a_sharp_reversal_costs_a_full_stop_and_start() {
        let there_and_back = polyline_op(&[(0.0, 0.0), (50.0, 0.0), (0.0, 0.0)], 3000.0);
        assert!((seconds(&there_and_back, CORNERING) - seconds(&there_and_back, REST_TO_REST)).abs() < 1e-9);
    }

    #[test]
    fn without_a_deviation_or_an_acceleration_the_old_estimates_stand() {
        let op = polyline_op(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 0.0)], 3000.0);
        let old = |a: f64| {
            let mut at = op.moves[0].target();
            op.moves
                .iter()
                .map(|m| {
                    let t = move_seconds(at, m, op.speed, 6000.0, a);
                    at = m.target();
                    t
                })
                .sum::<f64>()
        };
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let motion = Motion { acceleration: 500.0, junction_deviation: bad };
            assert_eq!(seconds(&op, motion), old(500.0), "deviation {bad} is ignored");
            let motion = Motion { acceleration: bad, junction_deviation: 0.01 };
            assert_eq!(seconds(&op, motion), old(bad), "acceleration {bad} is ignored");
        }
        let job = plan(&two_layer_doc(1)).unwrap();
        assert_eq!(
            Timeline::with_acceleration(&job, 6000.0, 200.0),
            Timeline::with_motion(&job, 6000.0, Motion { acceleration: 200.0, junction_deviation: 0.0 })
        );
    }

    #[test]
    fn moves_that_go_nowhere_take_no_time_and_do_not_break_a_straight_run() {
        let with_stops = polyline_op(&[(0.0, 0.0), (10.0, 0.0), (10.0, 0.0), (20.0, 0.0), (20.0, 0.0), (30.0, 0.0)], 3000.0);
        let plain = polyline_op(&[(0.0, 0.0), (30.0, 0.0)], 3000.0);
        let times = move_times(&with_stops, with_stops.moves[0].target(), 6000.0, CORNERING);
        assert_eq!(times.len(), with_stops.moves.len());
        assert_eq!(times[2], 0.0);
        assert_eq!(times[4], 0.0);
        assert!((times.iter().sum::<f64>() - seconds(&plain, CORNERING)).abs() < 1e-9);
    }

    #[test]
    fn planned_times_stay_between_the_speed_limit_and_the_rest_to_rest_estimate() {
        // A deterministic scribble of segments from a few hundredths of a millimetre to tens of millimetres.
        let mut seed = 12345_u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f64) / f64::from(1u32 << 31)
        };
        for speed in [600.0, 3000.0, 12000.0] {
            let mut points = vec![(0.0, 0.0)];
            for _ in 0..200 {
                let (x, y) = *points.last().unwrap();
                let scale = [0.05, 0.5, 5.0, 40.0][(next() * 4.0) as usize % 4];
                points.push((x + (next() - 0.5) * scale, y + (next() - 0.5) * scale));
            }
            let op = polyline_op(&points, speed);
            let length: f64 = points.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum();
            let (planned, from_rest) = (seconds(&op, CORNERING), seconds(&op, REST_TO_REST));
            assert!(planned.is_finite() && planned > 0.0, "{speed}: {planned}");
            assert!(planned <= from_rest + 1e-9, "{speed}: {planned} > {from_rest}");
            assert!(planned >= length / (speed / 60.0) - 1e-9, "{speed}: {planned} is quicker than the speed limit");
        }
    }

    #[test]
    fn acceleration_makes_short_moves_slower_per_millimetre_than_long_ones() {
        let cut = |x: f64| Move::Cut { to: Point::new(x, 0.0), power: 50.0 };
        let (speed, accel) = (3000.0, 500.0); // 50 mm/s, 500 mm/s²: 2.5 mm to reach speed
        let none = |d: f64| move_seconds(Point::ORIGIN, &cut(d), speed, 6000.0, 0.0);
        let with = |d: f64| move_seconds(Point::ORIGIN, &cut(d), speed, 6000.0, accel);
        assert!((none(100.0) - 2.0).abs() < 1e-12);
        assert!((with(100.0) - (2.0 + 0.1)).abs() < 1e-9, "a long move adds the time to speed up and slow down: {}", with(100.0));
        assert!((with(1.0) - 2.0 * (1.0_f64 / 500.0).sqrt()).abs() < 1e-12, "a 1 mm move never reaches speed");
        assert!(with(1.0) / 1.0 > with(100.0) / 100.0, "short moves cost more per millimetre");
        assert!(with(1e-9) < 1e-3 && with(0.0) == 0.0);
        for bad in [f64::NAN, -5.0, f64::INFINITY] {
            assert_eq!(move_seconds(Point::ORIGIN, &cut(100.0), speed, 6000.0, bad), none(100.0), "{bad} is ignored");
        }
    }

    #[test]
    fn a_job_with_acceleration_takes_longer_but_cuts_the_same_distance() {
        let job = plan(&two_layer_doc(1)).unwrap();
        let (plain, accel) = (Timeline::new(&job, 6000.0), Timeline::with_acceleration(&job, 6000.0, 200.0));
        assert!(accel.total > plain.total);
        assert!((accel.cut_mm() - plain.cut_mm()).abs() < 1e-9);
    }

    #[test]
    fn totals_match_the_estimate_and_layers_add_up() {
        let job = plan(&two_layer_doc(2)).unwrap();
        let t = Timeline::new(&job, 6000.0);
        let e = estimate(&job, 6000.0);
        assert!((t.total - e.seconds).abs() < 1e-9 && (t.cut_mm() - e.cut_mm).abs() < 1e-9);
        let layers = t.layers();
        assert_eq!(layers.len(), 2);
        assert!((layers.iter().map(|l| l.seconds).sum::<f64>() - t.total).abs() < 1e-9);
        assert!(layers[1].seconds > layers[0].seconds, "the 30 mm square at the same speed takes longer than the 10 mm one");
    }

    #[test]
    fn at_finds_the_operation_and_pass() {
        let job = plan(&two_layer_doc(3)).unwrap();
        let t = Timeline::new(&job, 6000.0);
        let first = t.ops[0];
        let m = t.at(first.start + first.pass_seconds * 1.5).unwrap();
        assert_eq!((m.op, m.pass), (0, 1));
        assert!((m.into_pass - first.pass_seconds * 0.5).abs() < 1e-9);
        let second = t.ops[1];
        assert_eq!(t.at(second.start + 0.001).unwrap().op, 1);
        let end = t.at(t.total).unwrap();
        assert_eq!((end.op, end.pass), (1, 2), "the end is the last pass of the last operation");
        assert_eq!(t.at(0.0).unwrap().op, 0);
    }

    #[test]
    fn hostile_times_and_empty_jobs_are_handled() {
        let t = Timeline::new(&plan(&two_layer_doc(1)).unwrap(), 6000.0);
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -5.0, 1e300] {
            assert!(t.at(x).is_some(), "{x}");
        }
        assert_eq!(Timeline::new(&Job::default(), 6000.0).at(1.0), None);
        let bad = Timeline::new(&plan(&two_layer_doc(1)).unwrap(), f64::NAN);
        assert!(bad.total.is_finite() && bad.total > 0.0, "a bad rapid speed falls back to a default");
    }

    fn two_layer_doc(passes: u32) -> Document {
        let mut doc = rect_doc(10.0, 10.0, LayerMode::Line);
        let second = 1;
        for id in [0, second] {
            if let Some(l) = doc.layer_mut(id) {
                l.mode = LayerMode::Line;
                l.speed = 600.0;
                l.passes = passes;
            }
        }
        doc.add_shape(Shape::path("big", second, rect_path(50.0, 10.0, 30.0, 30.0)));
        doc
    }
}

mod order_option_tests {
    use laserfoundry_doc::Optimize;

    use super::*;

    #[test]
    fn a_shared_edge_is_cut_once() {
        let rect = |x0: f64, x1: f64| Polyline::new(vec![Point::new(x0, 0.0), Point::new(x1, 0.0), Point::new(x1, 10.0), Point::new(x0, 10.0)], true);
        let length = |ls: &[Polyline]| -> f64 { ls.iter().map(|l| l.length()).sum() };
        let both = vec![rect(0.0, 10.0), rect(10.0, 20.0)];
        let before = length(&both);
        let after = remove_overlaps(both);
        assert!((before - after.iter().map(|l| l.length()).sum::<f64>() - 10.0).abs() < 1e-6, "the shared 10 mm side is cut once");
        assert!((length(&after) - 70.0).abs() < 1e-6);
        let apart = vec![rect(0.0, 10.0), rect(11.0, 20.0)];
        assert_eq!(remove_overlaps(apart.clone()), apart, "shapes that do not touch are untouched");
    }

    #[test]
    fn partly_overlapping_collinear_lines_are_trimmed() {
        let line = |a: f64, b: f64| Polyline::new(vec![Point::new(a, 0.0), Point::new(b, 0.0)], false);
        let out = remove_overlaps(vec![line(0.0, 10.0), line(5.0, 15.0), line(2.0, 8.0)]);
        assert_eq!(out, vec![line(0.0, 10.0), line(10.0, 15.0)]);
    }

    #[test]
    fn overlap_removal_ignores_hostile_input() {
        let nan = Polyline::new(vec![Point::new(f64::NAN, 0.0), Point::new(1.0, 0.0)], false);
        assert_eq!(remove_overlaps(vec![nan.clone(), nan]).len(), 2);
        let dot = Polyline::new(vec![Point::new(1.0, 1.0), Point::new(1.0, 1.0)], false);
        assert_eq!(remove_overlaps(vec![dot.clone(), dot.clone()]).len(), 2);
        assert!(remove_overlaps(Vec::new()).is_empty());
        let many: Vec<Polyline> = (0..4000).map(|i| Polyline::new(vec![Point::new(0.0, 0.0), Point::new(f64::from(i), 0.0)], false)).collect();
        assert_eq!(remove_overlaps(many).len(), 4000, "too many segments are left alone");
    }

    #[test]
    fn identical_outlines_are_cut_once_whatever_their_direction_or_start() {
        let sq = |pts: [(f64, f64); 4]| Polyline::new(pts.iter().map(|&(x, y)| Point::new(x, y)).collect(), true);
        let a = sq([(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let reversed = sq([(0.0, 10.0), (10.0, 10.0), (10.0, 0.0), (0.0, 0.0)]);
        let rotated = sq([(10.0, 10.0), (0.0, 10.0), (0.0, 0.0), (10.0, 0.0)]);
        let other = sq([(0.0, 0.0), (11.0, 0.0), (11.0, 10.0), (0.0, 10.0)]);
        let kept = remove_duplicates(vec![a.clone(), reversed, rotated, other.clone()]);
        assert_eq!(kept, vec![a, other]);

        let open = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(5.0, 5.0), Point::new(9.0, 1.0)], false);
        let open_back = Polyline::new(open.points.iter().rev().copied().collect(), false);
        assert_eq!(remove_duplicates(vec![open.clone(), open_back]).len(), 1);
        let closed_same_points = Polyline::new(open.points.clone(), true);
        assert_eq!(remove_duplicates(vec![open, closed_same_points]).len(), 2, "open and closed are different cuts");
    }

    #[test]
    fn hostile_coordinates_never_panic_or_vanish() {
        let bad = Polyline::new(vec![Point::new(f64::NAN, 0.0), Point::new(1.0, f64::INFINITY)], false);
        let huge = Polyline::new(vec![Point::new(1e300, 0.0), Point::new(0.0, -1e300)], false);
        assert_eq!(remove_duplicates(vec![bad.clone(), bad, huge.clone(), huge]).len(), 4, "lines it cannot compare are left alone");
        assert!(remove_duplicates(Vec::new()).is_empty());
        assert_eq!(remove_duplicates(vec![Polyline::new(Vec::new(), true), Polyline::new(Vec::new(), true)]).len(), 1);
    }

    #[test]
    fn planning_drops_a_pasted_copy_unless_the_option_is_off() {
        let mut doc = rect_doc(20.0, 20.0, LayerMode::Line);
        doc.add_shape(Shape::path("copy", 0, rect_path(10.0, 10.0, 20.0, 20.0)));
        let cut_moves = |doc: &Document| plan(doc).unwrap().ops.iter().flat_map(|o| &o.moves).filter(|m| matches!(m, Move::Cut { .. })).count();
        let once = cut_moves(&doc);
        doc.optimize.remove_duplicates = false;
        assert_eq!(cut_moves(&doc), once * 2);
    }

    #[test]
    fn a_duplicate_pair_does_not_cancel_itself_out_of_a_fill() {
        let mut doc = rect_doc(20.0, 20.0, LayerMode::Fill);
        doc.add_shape(Shape::path("copy", 0, rect_path(10.0, 10.0, 20.0, 20.0)));
        let moves = plan(&doc).unwrap().ops.iter().map(|o| o.moves.len()).sum::<usize>();
        assert!(moves > 0, "the square is still engraved");
    }

    #[test]
    fn a_lower_cut_priority_is_finished_first_whatever_the_drawing_order() {
        let mut doc = rect_doc(10.0, 10.0, LayerMode::Line);
        let mut second = doc.shapes[0].clone();
        second.transform = laserfoundry_geom::Affine::translate((50.0, 0.0));
        let late = doc.add_shape(second);
        let first_x = |doc: &Document| match plan(doc).unwrap().ops[0].moves[0] {
            Move::Travel(p) => p.x,
            m => panic!("{m:?}"),
        };
        let drawn_first = first_x(&doc);
        assert!(drawn_first < 20.0, "{drawn_first}");
        doc.shape_mut(late).unwrap().order = -1;
        assert!(first_x(&doc) > 40.0, "the later shape goes first");
        doc.shape_mut(late).unwrap().order = 5;
        assert!(first_x(&doc) < 20.0);
    }

    #[test]
    fn ordering_options_choose_between_depth_drawing_order_and_nearest() {
        let rect = |x: f64, y: f64, w: f64| {
            Polyline::new(vec![Point::new(x, y), Point::new(x + w, y), Point::new(x + w, y + w), Point::new(x, y + w)], true)
        };
        let outer = rect(0.0, 0.0, 100.0);
        let inner = rect(40.0, 40.0, 10.0);
        let far = rect(300.0, 0.0, 10.0);
        let lines = vec![outer.clone(), far.clone(), inner.clone()];
        let start = Point::new(0.0, 0.0);
        let order = |inner_first, reduce_travel| {
            let opts = Optimize { inner_first, reduce_travel, remove_duplicates: false, ..Optimize::default() };
            order_polylines(lines.clone(), start, &opts).iter().map(|l| l.bounds().map_or(0.0, |b| b.x1 - b.x0).round() as usize).collect::<Vec<_>>()
        };
        let sizes = |v: Vec<usize>| v;
        // Default: the hole-like inner square first, then the rest by distance.
        assert_eq!(sizes(order(true, true))[0], 10);
        // Neither option: exactly the drawing order.
        assert_eq!(sizes(order(false, false)), vec![100, 10, 10]);
        // Depth only: inner first, the rest keep their drawing order.
        assert_eq!(sizes(order(true, false)), vec![10, 100, 10]);
        // Nearest only: starting at the origin the outer square is nearest.
        assert_eq!(sizes(order(false, true))[0], 100);
    }
}

mod perforation_tests {
    use super::*;
    use crate::perforate::perforate;

    fn line(len: f64) -> Vec<Point> {
        vec![Point::new(0.0, 0.0), Point::new(len, 0.0)]
    }

    fn length(points: &[Point]) -> f64 {
        points.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    #[test]
    fn a_line_alternates_cut_and_skip_pieces_and_covers_the_whole_length() {
        let pieces = perforate(&line(50.0), false, 4.0, 1.0).unwrap();
        assert_eq!(pieces.len(), 20, "ten 5 mm periods");
        assert!(pieces.iter().enumerate().all(|(i, p)| p.on == (i % 2 == 0)));
        assert!((pieces.iter().map(|p| length(&p.points)).sum::<f64>() - 50.0).abs() < 1e-6);
        assert!((length(&pieces[0].points) - 4.0).abs() < 1e-9 && (length(&pieces[1].points) - 1.0).abs() < 1e-9);
        for w in pieces.windows(2) {
            assert!(w[0].points.last().unwrap().distance(w[1].points[0]) < 1e-9, "pieces join up");
        }
    }

    #[test]
    fn dashes_follow_corners_and_closed_outlines_include_the_closing_edge() {
        let square = vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
        let pieces = perforate(&square, true, 4.0, 3.0).unwrap();
        let total: f64 = pieces.iter().map(|p| length(&p.points)).sum();
        assert!((total - 40.0).abs() < 1e-6, "{total}");
        assert!(pieces.iter().any(|p| p.points.len() > 2), "a dash can turn a corner");
        let open = perforate(&square, false, 4.0, 3.0).unwrap();
        assert!((open.iter().map(|p| length(&p.points)).sum::<f64>() - 30.0).abs() < 1e-6);
    }

    #[test]
    fn unusable_lengths_and_inputs_give_none() {
        for (cut, gap) in [(0.0, 1.0), (1.0, 0.0), (0.01, 1.0), (f64::NAN, 1.0), (1.0, f64::INFINITY), (-1.0, 1.0)] {
            assert!(perforate(&line(10.0), false, cut, gap).is_none(), "{cut} {gap}");
        }
        assert!(perforate(&[], false, 1.0, 1.0).is_none());
        assert!(perforate(&[Point::new(0.0, 0.0)], true, 1.0, 1.0).is_none());
        assert!(perforate(&line(1e9), false, 0.05, 0.05).is_none(), "too many pieces to be sensible");
        assert!(perforate(&line(f64::INFINITY), false, 1.0, 1.0).is_none());
        let degenerate = vec![Point::new(1.0, 1.0); 5];
        assert!(
            perforate(&degenerate, true, 1.0, 1.0).is_some_and(|p| p.iter().all(|x| x.points.len() <= 2)),
            "zero-length edges do not loop forever"
        );
    }

    #[test]
    fn crosshatch_fills_twice_at_right_angles() {
        let mut doc = rect_doc(30.0, 30.0, LayerMode::Fill);
        if let Some(l) = doc.layer_mut(0) {
            l.interval = 1.0;
        }
        let single = plan(&doc).unwrap();
        assert_eq!(single.ops.len(), 1);
        if let Some(l) = doc.layer_mut(0) {
            l.crosshatch = true;
        }
        let both = plan(&doc).unwrap();
        assert_eq!(both.ops.len(), 2);
        let direction = |op: &Op| match (op.moves.first(), op.moves.get(1)) {
            (Some(Move::Travel(a)), Some(Move::Cut { to, .. })) => (to.x - a.x).abs() > (to.y - a.y).abs(),
            other => panic!("{other:?}"),
        };
        assert_ne!(direction(&both.ops[0]), direction(&both.ops[1]), "one pass runs along x, the other along y");
        assert!((estimate(&both, 6000.0).cut_mm / estimate(&single, 6000.0).cut_mm - 2.0).abs() < 0.2);
    }

    #[test]
    fn a_perforated_layer_uses_rapid_moves_over_the_gaps_and_cuts_half_the_length() {
        let mut doc = rect_doc(40.0, 40.0, LayerMode::Line);
        if let Some(l) = doc.layer_mut(0) {
            l.perf_cut = 5.0;
            l.perf_gap = 5.0;
            l.tab_count = 4; // perforation wins over tabs
        }
        let job = plan(&doc).unwrap();
        let moves = &job.ops[0].moves;
        let travels = moves.iter().filter(|m| matches!(m, Move::Travel(_))).count();
        assert!(travels >= 16, "one rapid move per gap plus the first: {travels}");
        let e = estimate(&job, 6000.0);
        assert!((e.cut_mm - 80.0).abs() < 1.0, "{}", e.cut_mm);
        assert!(moves.iter().all(|m| !matches!(m, Move::Cut { power, .. } if *power == 0.0)), "no zero-power cuts: gaps are travels");
    }

    #[test]
    fn bad_perforation_settings_are_a_plan_error_not_a_panic() {
        for bad in [f64::NAN, -1.0, 5000.0] {
            let mut doc = rect_doc(10.0, 10.0, LayerMode::Line);
            if let Some(l) = doc.layer_mut(0) {
                l.perf_cut = bad;
            }
            assert!(matches!(plan(&doc), Err(PlanError::BadLayerSetting { .. })), "{bad}");
        }
    }
}

mod trace_tests {
    use super::*;

    fn picture(rows: &[&str]) -> (usize, usize, Vec<bool>) {
        let h = rows.len();
        let w = rows.first().map_or(0, |r| r.len());
        (w, h, rows.iter().flat_map(|r| r.bytes().map(|b| b == b'#')).collect())
    }

    fn run(rows: &[&str], tol: f64, min_area: f64) -> Vec<Polyline> {
        let (w, h, px) = picture(rows);
        trace(w, h, |x, y| px[y * w + x], tol, min_area).unwrap()
    }

    fn area(l: &Polyline) -> f64 {
        let p = &l.points;
        (0..p.len()).map(|i| p[i].x * p[(i + 1) % p.len()].y - p[(i + 1) % p.len()].x * p[i].y).sum::<f64>() / 2.0
    }

    #[test]
    fn a_solid_block_becomes_one_rectangle() {
        let loops = run(&["....", ".##.", ".##.", "...."], 0.5, 0.0);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].points.len(), 4, "{:?}", loops[0].points);
        assert!((area(&loops[0]).abs() - 4.0).abs() < 1e-9);
        let b = loops[0].bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (1.0, 1.0, 3.0, 3.0));
    }

    #[test]
    fn a_ring_has_an_outer_loop_and_an_opposite_hole() {
        let loops = run(&["#####", "#...#", "#...#", "#...#", "#####"], 0.5, 0.0);
        assert_eq!(loops.len(), 2);
        let (outer, hole) = if area(&loops[0]).abs() > area(&loops[1]).abs() { (&loops[0], &loops[1]) } else { (&loops[1], &loops[0]) };
        assert!((area(outer).abs() - 25.0).abs() < 1e-9 && (area(hole).abs() - 9.0).abs() < 1e-9);
        assert!(area(outer) * area(hole) < 0.0, "the hole runs the other way");
    }

    #[test]
    fn separate_blobs_specks_and_diagonal_neighbours() {
        let loops = run(&["##...#", "##....", ".....#"], 0.5, 0.0);
        assert_eq!(loops.len(), 3);
        assert_eq!(run(&["##...#", "##....", ".....#"], 0.5, 2.0).len(), 1, "specks under the minimum area are dropped");
        let diag = run(&["#.", ".#"], 0.0, 0.0);
        let total: f64 = diag.iter().map(|l| area(l).abs()).sum();
        assert!((total - 2.0).abs() < 1e-9, "touching corners lose no pixels: {diag:?}");
    }

    #[test]
    fn staircases_simplify_to_a_diagonal_within_the_tolerance() {
        let rows: Vec<String> = (0..20).map(|y| format!("{}{}", "#".repeat(y + 1), ".".repeat(19 - y))).collect();
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let exact = run(&refs, 0.0, 0.0);
        let loose = run(&refs, 1.0, 0.0);
        assert!(loose[0].points.len() < 8 && exact[0].points.len() > 30, "{} vs {}", loose[0].points.len(), exact[0].points.len());
    }

    #[test]
    fn tracing_gives_the_same_answer_every_time() {
        let rows = ["..####..", ".######.", "########", "########", ".######.", "..####..", "#......#", ".#....#."];
        let first = run(&rows, 1.0, 0.0);
        for _ in 0..20 {
            assert_eq!(run(&rows, 1.0, 0.0), first);
        }
    }

    #[test]
    fn empty_oversized_and_hostile_inputs_are_handled() {
        assert!(trace(0, 5, |_, _| true, 1.0, 0.0).is_none());
        assert!(trace(5, 0, |_, _| true, 1.0, 0.0).is_none());
        assert!(trace(5000, 5000, |_, _| true, 1.0, 0.0).is_none(), "25 million pixels is too many");
        assert!(trace(usize::MAX, 2, |_, _| true, 1.0, 0.0).is_none());
        assert!(trace(4, 4, |_, _| false, 1.0, 0.0).unwrap().is_empty());
        assert_eq!(trace(4, 4, |_, _| true, f64::NAN, f64::NAN).unwrap().len(), 1);
        // A checkerboard is the worst case for edge count and must still finish.
        assert!(trace(200, 200, |x, y| (x + y) % 2 == 0, 1.0, 0.0).is_some());
    }
}

mod overscan_and_ramp_tests {
    use super::*;

    fn fill_doc() -> Document {
        let mut doc = rect_doc(40.0, 20.0, LayerMode::Fill);
        if let Some(l) = doc.layer_mut(0) {
            l.interval = 5.0;
        }
        doc
    }

    #[test]
    fn overscan_can_follow_the_layer_speed_and_the_machine_acceleration() {
        let mut doc = fill_doc();
        if let Some(l) = doc.layer_mut(0) {
            l.speed = 6000.0; // 100 mm/s
        }
        doc.machine.acceleration = 1000.0; // 100² / 2000 = 5 mm
        doc.machine.overscan_by_speed = true;
        let job = plan(&doc).unwrap();
        let Move::Travel(start) = job.ops[0].moves[0] else { panic!() };
        let Move::Cut { to: line_start, .. } = job.ops[0].moves[1] else { panic!() };
        assert!((line_start.x - start.x).abs() - 5.0 < 1e-9 && (line_start.x - start.x).abs() > 4.99, "{start:?} {line_start:?}");
        if let Some(l) = doc.layer_mut(0) {
            l.overscan = Some(0.0);
        }
        assert_eq!(
            plan(&doc).unwrap().ops[0].moves.len(),
            plan(&{
                let mut d = doc.clone();
                d.machine.overscan_by_speed = false;
                d
            })
            .unwrap()
            .ops[0]
                .moves
                .len(),
            "a layer's own value wins"
        );
        for bad in [0.0, f64::NAN, f64::INFINITY] {
            let mut d = fill_doc();
            d.machine.overscan_by_speed = true;
            d.machine.acceleration = bad;
            assert!(plan(&d).is_ok(), "{bad}");
        }
    }

    #[test]
    fn fill_lines_get_a_run_up_and_run_out_with_the_laser_off() {
        let mut doc = fill_doc();
        let plain = plan(&doc).unwrap();
        if let Some(l) = doc.layer_mut(0) {
            l.overscan = Some(3.0);
        }
        let with = plan(&doc).unwrap();
        let (a, b) = (&plain.ops[0].moves, &with.ops[0].moves);
        assert_eq!(b.len(), a.len() * 2, "two extra moves per scan line");
        // First line: travel to 3 mm before the start, then a silent run-up, the burn, a silent run-out.
        let Move::Travel(start) = b[0] else { panic!("{:?}", b[0]) };
        let Move::Travel(line_start) = a[0] else { panic!("{:?}", a[0]) };
        assert!((line_start.x - start.x - 3.0).abs() < 1e-9 || (start.x - line_start.x - 3.0).abs() < 1e-9, "{start:?} vs {line_start:?}");
        assert!(matches!(b[1], Move::Cut { power, .. } if power == 0.0));
        assert!(matches!(b[2], Move::Cut { power, .. } if power > 0.0));
        assert!(matches!(b[3], Move::Cut { power, .. } if power == 0.0));
        let burned = |m: &[Move]| estimate(&Job { ops: vec![Op { moves: m.to_vec(), ..plain.ops[0].clone() }] }, 6000.0).cut_mm;
        assert!(burned(b) > burned(a), "the run-up and run-out are travelled at feed speed");
    }

    #[test]
    fn overscan_never_leaves_the_bed_with_absolute_placement_and_a_layer_value_beats_the_machine() {
        let mut doc = rect_doc(40.0, 20.0, LayerMode::Fill);
        doc.shapes[0].transform = Affine::translate((-9.9, 0.0)); // the rectangle now starts at x = 0.1
        if let Some(l) = doc.layer_mut(0) {
            l.interval = 5.0;
            l.overscan = Some(10.0);
        }
        for op in plan(&doc).unwrap().ops {
            for m in op.moves {
                let p = m.target();
                assert!(p.x >= -1e-9 && p.x <= doc.machine.bed_width + 1e-9, "{p:?}");
            }
        }
        let mut img = rect_doc(1.0, 1.0, LayerMode::Fill);
        img.machine.overscan = 7.0;
        if let Some(l) = img.layer_mut(0) {
            l.overscan = Some(0.0);
        }
        assert_eq!(img.layer(0).unwrap().overscan, Some(0.0));
        let mut bad = fill_doc();
        for v in [f64::NAN, -1.0, 500.0] {
            if let Some(l) = bad.layer_mut(0) {
                l.overscan = Some(v);
            }
            assert!(matches!(plan(&bad), Err(PlanError::BadLayerSetting { .. })), "{v}");
        }
    }

    #[test]
    fn a_ramp_raises_power_from_the_minimum_over_the_ramp_length() {
        let mut doc = rect_doc(40.0, 20.0, LayerMode::Line);
        if let Some(l) = doc.layer_mut(0) {
            l.min_power = 10.0;
            l.max_power = 50.0;
            l.ramp_length = 10.0;
        }
        let moves = plan(&doc).unwrap().ops.remove(0).moves;
        let powers: Vec<f64> = moves.iter().filter_map(|m| if let Move::Cut { power, .. } = m { Some(*power) } else { None }).collect();
        assert!(powers[0] > 10.0 && powers[0] < 15.0, "starts near the minimum: {}", powers[0]);
        assert!(powers.windows(2).take(15).all(|w| w[1] >= w[0]), "rises steadily");
        assert!(powers.iter().any(|p| (*p - 50.0).abs() < 1e-9), "reaches full power");
        assert!(powers.iter().all(|p| *p <= 50.0 + 1e-9 && *p >= 10.0 - 1e-9));
        // Same path, same length.
        let mut flat = doc.clone();
        if let Some(l) = flat.layer_mut(0) {
            l.ramp_length = 0.0;
        }
        let (a, b) = (estimate(&plan(&doc).unwrap(), 6000.0), estimate(&plan(&flat).unwrap(), 6000.0));
        assert!((a.cut_mm - b.cut_mm).abs() < 1e-6, "{} vs {}", a.cut_mm, b.cut_mm);
    }

    #[test]
    fn ramps_handle_short_paths_zero_length_edges_and_bad_values() {
        let mut doc = rect_doc(2.0, 2.0, LayerMode::Line);
        if let Some(l) = doc.layer_mut(0) {
            l.ramp_length = 100.0; // longer than the whole outline
        }
        let moves = plan(&doc).unwrap().ops.remove(0).moves;
        assert!(moves.iter().all(|m| matches!(m, Move::Travel(_) | Move::Cut { .. })));
        for v in [f64::NAN, -1.0, 5000.0] {
            if let Some(l) = doc.layer_mut(0) {
                l.ramp_length = v;
            }
            assert!(matches!(plan(&doc), Err(PlanError::BadLayerSetting { .. })), "{v}");
        }
        if let Some(l) = doc.layer_mut(0) {
            l.ramp_length = 5.0;
            l.min_power = 90.0;
            l.max_power = 20.0; // minimum above maximum: no ramp, no panic
        }
        assert!(plan(&doc).is_ok());
    }
}

mod group_and_start_tests {
    use super::*;

    /// Three squares in a row at x = 10, 100 and 200; the first and last belong to one group.
    fn row() -> Document {
        let mut doc = Document::default();
        for (i, x) in [10.0, 100.0, 200.0].into_iter().enumerate() {
            let mut shape = Shape::path(format!("s{i}"), 0, rect_path(x, 10.0, 20.0, 20.0));
            shape.group = if i == 1 { Some(6) } else { Some(5) };
            doc.add_shape(shape);
        }
        doc
    }

    fn first_x(op: &Op) -> f64 {
        op.moves.first().map_or(f64::NAN, |m| m.target().x)
    }

    #[test]
    fn without_grouping_the_nearest_outline_goes_next_and_with_it_each_group_finishes_first() {
        let mut doc = row();
        doc.optimize.by_group = false;
        let flat = plan(&doc).unwrap();
        assert_eq!(flat.ops.len(), 1);
        let xs: Vec<f64> = flat.ops[0].moves.iter().filter_map(|m| if let Move::Travel(p) = m { Some(p.x.round()) } else { None }).collect();
        assert!(xs[0] < 40.0 && xs[1] > 90.0 && xs[1] < 130.0 && xs[2] > 190.0, "nearest first: {xs:?}");

        doc.optimize.by_group = true;
        let grouped = plan(&doc).unwrap();
        assert_eq!(grouped.ops.len(), 2, "one operation per group");
        // Group 5 (x = 10 and 200) is cut as a whole, then group 6 (x = 100).
        assert!(first_x(&grouped.ops[0]) < 40.0);
        let reach = grouped.ops[0].moves.iter().map(|m| m.target().x).fold(0.0, f64::max);
        assert!(reach > 200.0, "the first operation includes the far shape of its group");
        assert!(first_x(&grouped.ops[1]) > 90.0 && first_x(&grouped.ops[1]) < 130.0);
    }

    #[test]
    fn the_start_point_decides_which_outline_is_first() {
        let mut doc = row();
        doc.optimize.start = Some([210.0, 10.0]);
        let job = plan(&doc).unwrap();
        let first = job.ops[0].moves.iter().find_map(|m| if let Move::Travel(p) = m { Some(*p) } else { None }).unwrap();
        assert!(first.x > 190.0, "ordering began near the chosen point: {first:?}");
        for bad in [Some([f64::NAN, 0.0]), Some([0.0, f64::INFINITY])] {
            doc.optimize.start = bad;
            let job = plan(&doc).unwrap();
            assert!(
                job.ops[0].moves.iter().find_map(|m| if let Move::Travel(p) = m { Some(p.x) } else { None }).unwrap() < 40.0,
                "a bad start falls back to the origin"
            );
        }
    }
}
