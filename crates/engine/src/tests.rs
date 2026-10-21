use serde_json::{Value, json};

use crate::{EngineError, Session, commands, find_command};

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">
  <rect x="10" y="10" width="30" height="30" fill="none" stroke="#ff0000"/>
  <circle cx="70" cy="70" r="10" fill="none" stroke="#0000ff"/>
</svg>"##;

fn rect(s: &mut Session) -> u64 {
    let v = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
    v["id"].as_u64().unwrap()
}

mod select_by_size_tests {
    use super::*;

    fn sized(s: &mut Session, w: f64, h: f64) -> u64 {
        let v = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": w, "height": h})).unwrap();
        v["id"].as_u64().unwrap()
    }

    fn picked(s: &mut Session, params: Value) -> Vec<u64> {
        let v = s.execute("edit.select_by_size", &params).unwrap();
        v["selected"].as_array().unwrap().iter().map(|i| i.as_u64().unwrap()).collect()
    }

    #[test]
    fn selects_shapes_within_every_limit_given() {
        let mut s = Session::new();
        let (small, wide, tall, big) = (sized(&mut s, 5.0, 5.0), sized(&mut s, 50.0, 5.0), sized(&mut s, 5.0, 50.0), sized(&mut s, 50.0, 50.0));
        assert_eq!(picked(&mut s, json!({"max_width": 10})), vec![small, tall]);
        assert_eq!(picked(&mut s, json!({"min_width": 10})), vec![wide, big]);
        assert_eq!(picked(&mut s, json!({"max_width": 10, "max_height": 10})), vec![small]);
        assert_eq!(picked(&mut s, json!({"min_width": 10, "min_height": 10})), vec![big]);
        assert_eq!(picked(&mut s, json!({"min_height": 4, "max_height": 6})), vec![small, wide]);
        assert!(picked(&mut s, json!({"min_width": 500})).is_empty() && s.selection.is_empty(), "nothing matches: nothing is selected");
    }

    #[test]
    fn limits_are_inclusive() {
        let mut s = Session::new();
        let a = sized(&mut s, 10.0, 10.0);
        assert_eq!(picked(&mut s, json!({"max_width": 10})), vec![a]);
        assert_eq!(picked(&mut s, json!({"min_width": 10})), vec![a]);
        assert!(picked(&mut s, json!({"max_width": 9.9})).is_empty());
    }

    #[test]
    fn can_narrow_the_current_selection_instead_of_searching_everything() {
        let mut s = Session::new();
        let (a, b, c) = (sized(&mut s, 5.0, 5.0), sized(&mut s, 6.0, 6.0), sized(&mut s, 7.0, 7.0));
        s.execute("edit.select", &json!({"ids": [b, c]})).unwrap();
        assert_eq!(picked(&mut s, json!({"max_width": 6.5, "in_selection": true})), vec![b]);
        assert_eq!(picked(&mut s, json!({"max_width": 6.5})), vec![a, b], "without it, everything is searched");
    }

    #[test]
    fn a_group_is_judged_by_its_combined_size() {
        let mut s = Session::new();
        let a = sized(&mut s, 5.0, 5.0);
        let v = s.execute("shape.rectangle", &json!({"x": 100, "y": 0, "width": 5, "height": 5})).unwrap();
        let b = v["id"].as_u64().unwrap();
        let lone = sized(&mut s, 5.0, 5.0);
        s.execute("edit.select", &json!({"ids": [a, b]})).unwrap();
        s.execute("edit.group", &json!({})).unwrap();
        // Each member is 5 mm wide but together they span 105 mm.
        assert_eq!(picked(&mut s, json!({"max_width": 10})), vec![lone]);
        assert_eq!(picked(&mut s, json!({"min_width": 100})), vec![a, b]);
    }

    #[test]
    fn refuses_missing_negative_and_non_finite_limits_and_changes_nothing() {
        let mut s = Session::new();
        let a = sized(&mut s, 5.0, 5.0);
        s.execute("edit.select", &json!({"ids": [a]})).unwrap();
        for bad in [
            json!({}),
            json!({"in_selection": true}),
            json!({"min_width": -1}),
            json!({"max_height": 1e300}),
            json!({"min_width": "wide"}),
            json!([1]),
        ] {
            assert!(s.execute("edit.select_by_size", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.selection, vec![a]);
        assert!(!s.can_undo() || s.history().0.iter().all(|l| *l != "Select by Size"), "selecting is not an undo step");
    }
}

mod history_tests {
    use super::*;

    #[test]
    fn undo_and_redo_restore_document_and_selection() {
        let mut s = Session::new();
        let id = rect(&mut s);
        assert_eq!(s.selection, vec![id]);
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty() && s.selection.is_empty());
        s.execute("edit.redo", &Value::Null).unwrap();
        assert_eq!(s.doc.shapes.len(), 1);
    }

    #[test]
    fn history_lists_labelled_steps_and_jumps_to_any_of_them() {
        let mut s = Session::new();
        rect(&mut s);
        rect(&mut s);
        s.execute("edit.select_all", &Value::Null).unwrap();
        s.execute("edit.delete", &json!({})).unwrap();
        let h = s.execute("edit.history", &Value::Null).unwrap();
        let steps = h["steps"].as_array().unwrap();
        assert!(steps.iter().all(|l| !l.as_str().unwrap().is_empty()), "every step has a label: {steps:?}");
        let n = steps.len();
        assert_eq!(h["position"], n);

        s.execute("edit.history_goto", &json!({"position": 1})).unwrap();
        assert_eq!(s.doc.shapes.len(), 1, "after the first rectangle only");
        let h = s.execute("edit.history", &Value::Null).unwrap();
        assert_eq!((h["position"].as_u64(), h["steps"].as_array().unwrap().len()), (Some(1), n), "the later steps stay redoable");

        s.execute("edit.history_goto", &json!({"position": 0})).unwrap();
        assert!(s.doc.shapes.is_empty() && !s.can_undo());
        s.execute("edit.history_goto", &json!({"position": n})).unwrap();
        assert!(s.doc.shapes.is_empty() && !s.can_redo(), "back at the newest step");
        s.execute("edit.history_goto", &json!({"position": 2})).unwrap();
        assert_eq!(s.doc.shapes.len(), 2);
    }

    #[test]
    fn history_goto_rejects_positions_outside_the_history_and_changes_nothing() {
        let mut s = Session::new();
        rect(&mut s);
        for bad in [json!({"position": 2}), json!({"position": 1000000}), json!({"position": -1}), json!({}), json!({"position": "x"})] {
            assert!(s.execute("edit.history_goto", &bad).is_err(), "{bad}");
        }
        assert_eq!((s.doc.shapes.len(), s.history().1), (1, 1));
    }

    #[test]
    fn a_new_action_after_jumping_back_drops_the_later_steps() {
        let mut s = Session::new();
        rect(&mut s);
        rect(&mut s);
        s.execute("edit.history_goto", &json!({"position": 1})).unwrap();
        rect(&mut s);
        let (steps, position) = s.history();
        assert_eq!((steps.len(), position), (2, 2));
    }

    #[test]
    fn a_new_action_clears_redo() {
        let mut s = Session::new();
        rect(&mut s);
        s.execute("edit.undo", &Value::Null).unwrap();
        rect(&mut s);
        assert!(!s.can_redo());
    }

    #[test]
    fn failed_commands_leave_no_trace() {
        let mut s = Session::new();
        rect(&mut s);
        let before = s.doc.clone();
        assert!(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": -5, "height": 5})).is_err());
        assert_eq!(s.doc, before);
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty(), "the failed command must not have created an undo step");
    }

    #[test]
    fn batch_is_atomic_and_one_undo_step() {
        let mut s = Session::new();
        let bad = json!({"commands": [
            {"id": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 5, "height": 5}},
            {"id": "shape.rectangle", "params": {"width": "nope"}},
        ]});
        assert!(s.execute("command.batch", &bad).is_err());
        assert!(s.doc.shapes.is_empty());

        let good = json!({"commands": [
            {"id": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 5, "height": 5}},
            {"id": "shape.ellipse", "params": {"cx": 20, "cy": 20, "rx": 3, "ry": 3}},
        ]});
        s.execute("command.batch", &good).unwrap();
        assert_eq!(s.doc.shapes.len(), 2);
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty());
    }

    #[test]
    fn history_is_bounded() {
        let mut s = Session::new();
        for _ in 0..(crate::HISTORY_LIMIT + 20) {
            rect(&mut s);
        }
        let mut undone = 0;
        while s.can_undo() {
            s.execute("edit.undo", &Value::Null).unwrap();
            undone += 1;
        }
        assert_eq!(undone, crate::HISTORY_LIMIT);
    }

    #[test]
    fn history_is_bounded_by_memory_too() {
        let mut s = Session::new();
        for _ in 0..50 {
            rect(&mut s);
        }
        let one_step = s.doc.approx_bytes();
        assert!(one_step > 0);
        // Room for about five steps of a document this size: the oldest steps go first, the newest stay.
        s.set_history_bytes_for_test(one_step * 5);
        for _ in 0..30 {
            rect(&mut s);
        }
        let mut undone = 0;
        while s.can_undo() {
            s.execute("edit.undo", &Value::Null).unwrap();
            undone += 1;
        }
        assert!((1..=8).contains(&undone), "{undone} undo steps kept");
    }
}

mod edit_tests {
    use super::*;

    #[test]
    fn move_resize_rotate_change_bounds_as_expected() {
        let mut s = Session::new();
        let id = rect(&mut s);
        s.execute("edit.move", &json!({"dx": 5, "dy": -5})).unwrap();
        let b = s.doc.bounds_of(&[id]).unwrap();
        assert_eq!((b.x0, b.y0), (15.0, 5.0));
        s.execute("edit.resize", &json!({"width": 40, "lock_aspect": true})).unwrap();
        let b = s.doc.bounds_of(&[id]).unwrap();
        assert!((b.width() - 40.0).abs() < 1e-9 && (b.height() - 20.0).abs() < 1e-9);
        assert!((b.x0 - 15.0).abs() < 1e-9, "resize keeps the top-left fixed");
        s.execute("edit.rotate", &json!({"degrees": 90})).unwrap();
        let b = s.doc.bounds_of(&[id]).unwrap();
        assert!((b.width() - 20.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6);
    }

    #[test]
    fn align_and_flip() {
        let mut s = Session::new();
        let a = rect(&mut s);
        s.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 10, "height": 10})).unwrap();
        s.execute("edit.select_all", &Value::Null).unwrap();
        s.execute("edit.align", &json!({"to": "left"})).unwrap();
        let bs: Vec<_> = s.doc.shapes.iter().filter_map(|sh| sh.world_bounds()).collect();
        assert!(bs.iter().all(|b| (b.x0 - 10.0).abs() < 1e-9));
        s.execute("edit.flip", &json!({"ids": [a], "axis": "horizontal"})).unwrap();
        assert_eq!(s.doc.bounds_of(&[a]).map(|b| (b.x0.round(), b.x1.round())), Some((10.0, 30.0)));
    }

    #[test]
    fn editing_with_nothing_selected_is_an_error() {
        let mut s = Session::new();
        assert!(matches!(s.execute("edit.move", &json!({"dx": 1, "dy": 1})), Err(EngineError::NothingSelected)));
    }

    #[test]
    fn duplicate_selects_the_copy() {
        let mut s = Session::new();
        let a = rect(&mut s);
        s.execute("edit.duplicate", &json!({})).unwrap();
        assert_eq!(s.doc.shapes.len(), 2);
        assert_ne!(s.selection, vec![a]);
    }
}

mod override_and_test_grid_tests {
    use super::*;

    #[test]
    fn a_cut_override_changes_one_shape_and_can_be_cleared_and_merged() {
        let mut s = Session::new();
        let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("shape.rectangle", &json!({"x": 100, "y": 10, "width": 20, "height": 10})).unwrap();
        s.execute("shape.cut_override", &json!({"ids": [a], "speed": 250, "max_power": 85})).unwrap();
        s.execute("shape.cut_override", &json!({"ids": [a], "passes": 2})).unwrap();
        let c = s.doc.shape(a).unwrap().cut.unwrap();
        assert_eq!((c.speed, c.max_power, c.passes), (Some(250.0), Some(85.0), Some(2)), "later calls merge");
        s.execute("machine.set", &json!({"origin": "top_left"})).unwrap();
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
        let text = g["gcode"].as_str().unwrap();
        assert!(text.contains("F250") && text.contains("S850") && text.contains("F1000") && text.contains("S600"), "both settings appear\n{text}");
        s.execute("shape.cut_override", &json!({"ids": [a], "clear": true})).unwrap();
        assert!(s.doc.shape(a).unwrap().cut.is_none());
        for bad in [json!({"ids": [a], "speed": 0}), json!({"ids": [a], "max_power": 150}), json!({"ids": [a], "min_power": "x"}), json!({"ids": []})]
        {
            assert!(s.execute("shape.cut_override", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_test_grid_varies_speed_across_and_power_down_with_labels() {
        let mut s = Session::new();
        let v = s
            .execute(
                "generate.test_grid",
                &json!({"cols": 4, "rows": 3, "speed_min": 200, "speed_max": 800, "power_min": 20, "power_max": 80, "size": 8, "gap": 2}),
            )
            .unwrap();
        assert_eq!(v["cells"], json!(12));
        let cells: Vec<_> = s.doc.shapes.iter().filter(|sh| sh.cut.is_some()).collect();
        assert_eq!(cells.len(), 12);
        let c = |r: usize, col: usize| cells[r * 4 + col].cut.unwrap();
        assert_eq!((c(0, 0).speed, c(0, 0).max_power), (Some(200.0), Some(20.0)));
        assert_eq!((c(0, 3).speed, c(2, 0).max_power), (Some(800.0), Some(80.0)));
        assert_eq!(c(1, 1).speed, Some(400.0), "evenly spaced");
        assert_eq!(c(1, 1).max_power, Some(50.0));
        let b = cells[5].world_bounds().unwrap();
        assert_eq!((b.x0, b.y0), (30.0, 30.0), "pitch is size + gap from the origin 20,20");
        let labels: Vec<_> = s.doc.shapes.iter().filter(|sh| matches!(sh.kind, laserfoundry_doc::ShapeKind::Text { .. })).collect();
        assert_eq!(labels.len(), 4 + 3, "one label per column and row");
        assert!(labels.iter().all(|l| l.layer == 1), "labels go on the next layer so they do not use the cell settings");
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty(), "one undo step");
    }

    #[test]
    fn a_test_grid_becomes_one_op_per_distinct_setting_in_the_gcode() {
        let mut s = Session::new();
        s.execute("machine.set", &json!({"origin": "top_left"})).unwrap();
        s.execute("layer.set", &json!({"id": 0, "mode": "fill", "interval": 1})).unwrap();
        s.execute(
            "generate.test_grid",
            &json!({"cols": 3, "rows": 2, "speed_min": 300, "speed_max": 900, "power_min": 30, "power_max": 60, "labels": false}),
        )
        .unwrap();
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
        let text = g["gcode"].as_str().unwrap();
        for f in ["F300", "F600", "F900"] {
            assert!(text.contains(f), "{f}");
        }
        for sv in ["S300", "S600"] {
            assert!(text.contains(sv), "{sv}");
        }
        assert_eq!(text.matches("pass 1").count(), 6, "six cells, six ops");
    }

    #[test]
    fn test_grids_are_bounded_and_validated() {
        let mut s = Session::new();
        for bad in [
            json!({"cols": 0}),
            json!({"cols": 21}),
            json!({"size": 0.1}),
            json!({"size": 1e9}),
            json!({"speed_min": 0}),
            json!({"power_max": 150}),
            json!({"layer": 99}),
            json!({"label_layer": 99}),
            json!({"x": f64::MAX}),
        ] {
            assert!(s.execute("generate.test_grid", &bad).is_err(), "{bad}");
        }
        assert!(s.doc.shapes.is_empty(), "failed grids leave nothing behind");
        assert!(s.execute("generate.test_grid", &json!({})).is_ok(), "defaults work and derive from the layer");
        assert_eq!(s.doc.shapes.iter().filter(|sh| sh.cut.is_some()).count(), 25);
    }

    #[test]
    fn overrides_survive_saving() {
        let mut s = Session::new();
        let id = s.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
        s.execute("shape.cut_override", &json!({"speed": 123, "max_power": 45})).unwrap();
        let back = laserfoundry_format::from_bytes(&laserfoundry_format::to_bytes(&s.doc).unwrap()).unwrap();
        assert_eq!(back.shape(id).unwrap().cut, s.doc.shape(id).unwrap().cut);
    }
}

mod node_edit_tests {
    use super::*;

    fn rect(s: &mut Session) -> u64 {
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn moving_a_node_reshapes_the_path_in_workspace_coordinates_even_after_the_shape_was_moved() {
        let mut s = Session::new();
        let id = rect(&mut s);
        s.execute("edit.move", &json!({"dx": 100, "dy": 50})).unwrap();
        s.execute("edit.rotate", &json!({"degrees": 30})).unwrap();
        // Drag the node nearest the top-left corner to an exact workspace point.
        let before = s.doc.shape(id).unwrap().world_polylines()[0].points.clone();
        s.execute("node.move", &json!({"id": id, "index": 0, "x": 90.0, "y": 40.0})).unwrap();
        let after = s.doc.shape(id).unwrap().world_polylines()[0].points.clone();
        assert!(after[0].distance(laserfoundry_geom::Point::new(90.0, 40.0)) < 1e-9, "{:?}", after[0]);
        for i in 1..4 {
            assert!(after[i].distance(before[i]) < 1e-9, "the other corners did not move");
        }
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shape(id).unwrap().world_polylines()[0].points[0].distance(before[0]) < 1e-9, "one undo step");
    }

    #[test]
    fn handles_inserts_deletes_and_conversions_change_the_path_as_asked() {
        let mut s = Session::new();
        let id = rect(&mut s);
        s.execute("node.set_segment", &json!({"id": id, "segment": 0, "curve": true})).unwrap();
        s.execute("node.move_handle", &json!({"id": id, "index": 0, "side": "out", "x": 20.0, "y": -5.0})).unwrap();
        let b = s.doc.shape(id).unwrap().world_bounds().unwrap();
        assert!(b.y0 < 10.0, "the top edge bulges upward: {b:?}");
        let v = s.execute("node.insert", &json!({"id": id, "segment": 1, "t": 0.5})).unwrap();
        assert_eq!(v["index"], json!(2));
        let nodes = |s: &Session| {
            let laserfoundry_doc::ShapeKind::Path { path } = &s.doc.shape(id).unwrap().kind else { panic!("not a path") };
            laserfoundry_geom::nodes::NodePath::from_bez(path).unwrap().node_count()
        };
        assert_eq!(nodes(&s), 5);
        s.execute("node.delete", &json!({"id": id, "index": 2})).unwrap();
        assert_eq!(nodes(&s), 4);
        s.execute("node.set_smooth", &json!({"id": id, "index": 1, "smooth": true})).unwrap();
        s.execute("node.set_closed", &json!({"id": id, "closed": false})).unwrap();
        assert!(!s.doc.shape(id).unwrap().world_polylines()[0].closed);
    }

    #[test]
    fn failures_are_atomic_and_explain_themselves() {
        let mut s = Session::new();
        let id = rect(&mut s);
        let before = s.doc.clone();
        for bad in [
            json!({"id": id, "index": 99, "x": 1, "y": 1}),
            json!({"id": id, "subpath": 3, "index": 0, "x": 1, "y": 1}),
            json!({"id": 9999, "index": 0, "x": 1, "y": 1}),
            json!({"id": id, "index": 0, "x": "a", "y": 1}),
            json!({"id": id, "index": 0, "x": 1e300, "y": 1}),
        ] {
            assert!(s.execute("node.move", &bad).is_err(), "{bad}");
        }
        assert!(s.execute("node.delete", &json!({"id": id, "index": 0})).is_ok(), "four nodes can lose one");
        assert!(s.execute("node.delete", &json!({"id": id, "index": 0})).is_err(), "but a closed path keeps three");
        assert!(s.execute("node.insert", &json!({"id": id, "segment": 40})).is_err());
        assert!(s.execute("node.set_segment", &json!({"id": id, "segment": 40, "curve": true})).is_err());
        assert_ne!(s.doc, before);
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc, before, "only the successful delete was an undo step");
    }

    #[test]
    fn text_must_be_converted_before_its_nodes_can_be_edited() {
        let mut s = Session::new();
        let id = s.execute("text.create", &json!({"text": "AB", "height": 10})).unwrap()["id"].as_u64().unwrap();
        assert!(s.execute("node.move", &json!({"id": id, "index": 0, "x": 1, "y": 1})).is_err());
        s.execute("edit.convert_to_path", &json!({})).unwrap();
        assert!(matches!(s.doc.shape(id).unwrap().kind, laserfoundry_doc::ShapeKind::Path { .. }));
        let b = s.doc.shape(id).unwrap().world_bounds().unwrap();
        s.execute("node.move", &json!({"id": id, "index": 0, "x": b.x0 + 5.0, "y": b.y0 + 5.0})).unwrap();
        assert!(s.execute("edit.convert_to_path", &json!({})).is_err(), "nothing left to convert");
        let rect_id = rect(&mut s);
        assert!(s.execute("edit.convert_to_path", &json!({"ids": [rect_id]})).is_err());
    }

    #[test]
    fn the_pen_creates_open_and_closed_paths_with_handles() {
        let mut s = Session::new();
        let v = s
            .execute(
                "path.create",
                &json!({"nodes": [
                    {"x": 10, "y": 10, "out": [20, 10], "smooth": true},
                    {"x": 40, "y": 30, "in": [30, 30]},
                    {"x": 10, "y": 50},
                ], "closed": true, "layer": 2}),
            )
            .unwrap();
        let id = v["id"].as_u64().unwrap();
        let shape = s.doc.shape(id).unwrap();
        assert_eq!(shape.layer, 2);
        assert!(shape.world_polylines()[0].closed);
        assert_eq!(s.selection, vec![id]);
        let b = shape.world_bounds().unwrap();
        assert!(b.x0 <= 10.0 && b.x1 >= 40.0 && b.y1 >= 50.0);
        for bad in [
            json!({"nodes": [{"x": 1, "y": 1}]}),
            json!({"nodes": [{"x": 1, "y": 1}, {"x": 2, "y": 2}], "closed": true}),
            json!({"nodes": [{"x": 1, "y": 1}, {"x": f64::NAN, "y": 2}]}),
            json!({"nodes": [{"x": 1, "y": 1}, {"x": 2, "y": 2, "out": [1e300, 0]}]}),
            json!({"nodes": [{"x": 1, "y": 1}, {"x": 2, "y": 2}], "layer": 99}),
            json!({"nodes": []}),
        ] {
            assert!(s.execute("path.create", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.doc.shapes.len(), 1);
        // A straight open two-node path works and engraves as a line.
        s.execute("path.create", &json!({"nodes": [{"x": 0, "y": 0}, {"x": 5, "y": 5}]})).unwrap();
        assert!(!s.doc.shapes[1].world_polylines()[0].closed);
    }
}

mod group_and_layer_tests {
    use super::*;

    fn rects(s: &mut Session, n: usize) -> Vec<u64> {
        (0..n)
            .map(|i| s.execute("shape.rectangle", &json!({"x": i as f64 * 30.0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap())
            .collect()
    }

    #[test]
    fn grouped_shapes_select_and_move_as_one() {
        let mut s = Session::new();
        let ids = rects(&mut s, 3);
        s.execute("edit.group", &json!({"ids": [ids[0], ids[1]]})).unwrap();
        s.execute("edit.select", &json!({"ids": [ids[0]]})).unwrap();
        assert_eq!(s.selection, vec![ids[0], ids[1]], "selecting one member selects the group");
        s.execute("edit.move", &json!({"dx": 5, "dy": 0})).unwrap();
        assert_eq!(s.doc.shape(ids[1]).unwrap().world_bounds().unwrap().x0, 35.0);
        assert_eq!(s.doc.shape(ids[2]).unwrap().world_bounds().unwrap().x0, 60.0, "the non-member stays");
        s.execute("edit.select", &json!({"ids": [ids[2]]})).unwrap();
        assert_eq!(s.selection, vec![ids[2]]);
    }

    #[test]
    fn ungroup_frees_the_members_and_group_needs_two_shapes() {
        let mut s = Session::new();
        let ids = rects(&mut s, 3);
        assert!(s.execute("edit.group", &json!({"ids": [ids[0]]})).is_err());
        assert!(s.execute("edit.ungroup", &json!({"ids": [ids[0]]})).is_err(), "nothing was grouped");
        s.execute("edit.group", &json!({"ids": ids})).unwrap();
        s.execute("edit.ungroup", &json!({"ids": [ids[1]]})).unwrap();
        s.execute("edit.select", &json!({"ids": [ids[0]]})).unwrap();
        assert_eq!(s.selection, vec![ids[0]]);
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("edit.select", &json!({"ids": [ids[0]]})).unwrap();
        assert_eq!(s.selection.len(), 3, "undo restores the group");
    }

    #[test]
    fn grouping_merges_existing_groups_and_copies_get_their_own() {
        let mut s = Session::new();
        let ids = rects(&mut s, 4);
        s.execute("edit.group", &json!({"ids": [ids[0], ids[1]]})).unwrap();
        s.execute("edit.group", &json!({"ids": [ids[1], ids[2]]})).unwrap();
        s.execute("edit.select", &json!({"ids": [ids[0]]})).unwrap();
        assert_eq!(s.selection.len(), 3, "the two groups merged");
        s.execute("edit.duplicate", &json!({"dx": 0, "dy": 40})).unwrap();
        let copies = s.selection.clone();
        assert_eq!(copies.len(), 3);
        s.execute("edit.move", &json!({"ids": [copies[0]], "dx": 1, "dy": 0})).unwrap();
        s.execute("edit.select", &json!({"ids": [ids[0]]})).unwrap();
        assert_eq!(s.selection.len(), 3, "the original group does not include the copies");
        s.execute("edit.select", &json!({"ids": [copies[1]]})).unwrap();
        assert_eq!(s.selection.len(), 3, "the copies are a group of their own");
        assert!(!s.selection.contains(&ids[0]));
        // Arrays of a group make a group per copy.
        s.execute("edit.select", &json!({"ids": [ids[0]]})).unwrap();
        s.execute("arrange.grid", &json!({"columns": 2, "rows": 1})).unwrap();
        let last_copy = *s.selection.last().unwrap();
        s.execute("edit.select", &json!({"ids": [last_copy]})).unwrap();
        assert_eq!(s.selection.len(), 3, "each array copy of a group is a group");
        assert!(!s.selection.contains(&ids[0]), "and does not include the original");
    }

    #[test]
    fn groups_survive_saving() {
        let mut s = Session::new();
        let ids = rects(&mut s, 2);
        s.execute("edit.group", &json!({"ids": ids})).unwrap();
        let back = laserfoundry_format::from_bytes(&laserfoundry_format::to_bytes(&s.doc).unwrap()).unwrap();
        assert_eq!(back.expand_groups(&[ids[0]]).len(), 2);
    }

    #[test]
    fn layers_can_be_renamed_recoloured_and_reordered_and_order_is_cut_order() {
        let mut s = Session::new();
        s.execute("machine.set", &json!({"origin": "top_left"})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10, "layer": 0})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 50, "y": 0, "width": 10, "height": 10, "layer": 1})).unwrap();
        let first_x = |s: &mut Session| {
            let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
            g["gcode"].as_str().unwrap().lines().find(|l| l.starts_with("G0 X")).unwrap().to_string()
        };
        assert!(first_x(&mut s).starts_with("G0 X0 ") || first_x(&mut s).starts_with("G0 X0"), "layer 0 is cut first");
        s.execute("layer.move", &json!({"id": 1, "index": 0})).unwrap();
        assert_eq!(s.doc.layers[0].id, 1);
        assert!(first_x(&mut s).starts_with("G0 X50"), "now layer 1 goes first");
        s.execute("layer.set", &json!({"id": 1, "name": "Cut outside", "color": "#AABBCC"})).unwrap();
        assert_eq!((s.doc.layer(1).unwrap().name.as_str(), s.doc.layer(1).unwrap().color.as_str()), ("Cut outside", "#aabbcc"));
        for bad in [
            json!({"id": 1, "color": "red"}),
            json!({"id": 1, "color": "#12345"}),
            json!({"id": 1, "color": "#gg0000"}),
            json!({"id": 1, "color": "#aabbccdd"}),
        ] {
            assert!(s.execute("layer.set", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.doc.layer(1).unwrap().color, "#aabbcc", "rejected colours change nothing");
        assert!(s.execute("layer.move", &json!({"id": 99, "index": 0})).is_err());
        assert!(s.execute("layer.move", &json!({"id": 1, "index": 16})).is_err());
        assert!(s.execute("layer.move", &json!({"id": 1, "index": -1})).is_err());
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.layers[0].id, 0, "reordering is undoable");
    }
}

mod array_tests {
    use super::*;

    fn square(s: &mut Session) -> u64 {
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn a_grid_array_lays_copies_out_by_size_plus_gap() {
        let mut s = Session::new();
        square(&mut s);
        let v = s.execute("arrange.grid", &json!({"columns": 3, "rows": 2, "gap_x": 4, "gap_y": 6})).unwrap();
        assert_eq!(v["count"], json!(6));
        assert_eq!(s.doc.shapes.len(), 6);
        let b = s.doc.bounds_of(&s.selection).unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 0.0, 38.0, 26.0), "3 squares + 2 gaps wide, 2 squares + 1 gap tall");
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.shapes.len(), 1, "one undo step");
    }

    #[test]
    fn a_circular_array_spaces_copies_evenly_and_keeps_the_radius() {
        let mut s = Session::new();
        square(&mut s);
        s.execute("arrange.circular", &json!({"count": 4, "radius": 50})).unwrap();
        assert_eq!(s.doc.shapes.len(), 4);
        // Array centre is 50 mm below the square's centre (5,5): (5,55). Every copy's centre is 50 mm from it.
        let centre = laserfoundry_geom::Point::new(5.0, 55.0);
        for sh in &s.doc.shapes {
            let c = sh.world_bounds().unwrap().center();
            assert!((c.distance(centre) - 50.0).abs() < 1e-6, "{c:?}");
        }
        let top = s.doc.shapes.iter().map(|sh| sh.world_bounds().unwrap().center().y).fold(f64::INFINITY, f64::min);
        assert!((top - 5.0).abs() < 1e-6, "the original stays where it was");
    }

    #[test]
    fn an_arc_array_runs_from_the_original_to_the_end_of_the_sweep() {
        let mut s = Session::new();
        square(&mut s);
        s.execute("arrange.circular", &json!({"count": 3, "radius": 40, "angle": 90})).unwrap();
        let centres: Vec<_> = s.doc.shapes.iter().map(|sh| sh.world_bounds().unwrap().center()).collect();
        let array_centre = laserfoundry_geom::Point::new(5.0, 45.0);
        let last = centres[2];
        let angle = (centres[0] - array_centre).atan2() - (last - array_centre).atan2();
        assert!((angle.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-6, "{angle}");
    }

    #[test]
    fn arrays_apply_to_whole_selections_and_are_bounded() {
        let mut s = Session::new();
        let a = square(&mut s);
        let b = s.execute("shape.ellipse", &json!({"cx": 30, "cy": 5, "rx": 5, "ry": 5})).unwrap()["id"].as_u64().unwrap();
        s.execute("arrange.grid", &json!({"ids": [a, b], "columns": 2, "rows": 1})).unwrap();
        assert_eq!(s.doc.shapes.len(), 4, "both shapes are copied together");
        for bad in [
            json!({"columns": 0, "rows": 3}),
            json!({"columns": 200, "rows": 200}),
            json!({"columns": 2, "rows": 2, "gap_x": 1e12}),
            json!({"columns": -1, "rows": 1}),
        ] {
            assert!(s.execute("arrange.grid", &bad).is_err(), "{bad}");
        }
        for bad in [json!({"count": 0}), json!({"count": 100000}), json!({"count": 3, "radius": "x"}), json!({"count": 3, "angle": null})] {
            assert!(s.execute("arrange.circular", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.doc.shapes.len(), 4, "failures leave the document unchanged or add nothing");
        let mut empty = Session::new();
        assert!(empty.execute("arrange.grid", &json!({"columns": 2, "rows": 2})).is_err());
    }

    #[test]
    fn rounded_rectangles_come_from_the_radius_parameter() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"width": 40, "height": 20, "radius": 5})).unwrap();
        let shape = &s.doc.shapes[0];
        assert_eq!(shape.name, "Rounded rectangle");
        assert!(shape.world_polylines()[0].closed && shape.world_polylines()[0].points.len() > 8);
        assert!(
            s.execute("shape.rectangle", &json!({"width": 4, "height": 4, "radius": f64::NAN})).is_err(),
            "NaN becomes null, which is not a number"
        );
        assert!(s.execute("shape.rectangle", &json!({"width": 4, "height": 4, "radius": 1e12})).is_err());
    }
}

mod nest_tests {
    use super::*;

    fn block(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> u64 {
        s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap()
    }

    fn bounds(s: &Session, id: u64) -> laserfoundry_geom::Rect {
        s.doc.shape(id).unwrap().world_bounds().unwrap()
    }

    /// True when the two shapes are at least `gap` mm apart (their boxes, which for rectangles is exact).
    fn apart(a: laserfoundry_geom::Rect, b: laserfoundry_geom::Rect, gap: f64) -> bool {
        let (dx, dy) = ((b.x0 - a.x1).max(a.x0 - b.x1), (b.y0 - a.y1).max(a.y0 - b.y1));
        dx.max(dy) >= gap - 1e-6
    }

    #[test]
    fn nesting_packs_the_selection_into_the_bed_and_one_undo_puts_it_back() {
        let mut s = Session::new();
        s.execute("machine.set", &json!({"bed_width": 100, "bed_height": 60})).unwrap();
        // Scattered around well outside the 100 x 60 bed, and one too big to share a row with the others.
        let ids: Vec<u64> =
            [(300.0, 300.0, 30.0, 20.0), (500.0, 40.0, 25.0, 25.0), (-80.0, 10.0, 40.0, 15.0), (200.0, 200.0, 12.0, 12.0), (0.0, 400.0, 18.0, 30.0)]
                .iter()
                .map(|&(x, y, w, h)| block(&mut s, x, y, w, h))
                .collect();
        let before: Vec<_> = ids.iter().map(|id| bounds(&s, *id)).collect();
        let result = s.execute("arrange.nest", &json!({"ids": ids, "gap": 3})).unwrap();
        assert_eq!(result["placed"], 5, "{result}");
        assert_eq!(result["unplaced"], json!([]));
        let after: Vec<_> = ids.iter().map(|id| bounds(&s, *id)).collect();
        for (i, a) in after.iter().enumerate() {
            assert!(a.x0 >= -1e-6 && a.y0 >= -1e-6 && a.x1 <= 100.0 + 1e-6 && a.y1 <= 60.0 + 1e-6, "{i} is off the bed: {a:?}");
            for (j, b) in after.iter().enumerate().skip(i + 1) {
                assert!(apart(*a, *b, 3.0), "{i} and {j} are too close: {a:?} {b:?}");
            }
        }
        assert_eq!(s.selection.len(), 5, "everything that was nested is selected");
        s.execute("edit.undo", &Value::Null).unwrap();
        let restored: Vec<_> = ids.iter().map(|id| bounds(&s, *id)).collect();
        assert_eq!(restored, before, "one undo step restores every position");
    }

    #[test]
    fn a_group_is_nested_as_one_part_and_keeps_its_shape() {
        let mut s = Session::new();
        s.execute("machine.set", &json!({"bed_width": 120, "bed_height": 80})).unwrap();
        let a = block(&mut s, 200.0, 200.0, 20.0, 10.0);
        let b = block(&mut s, 240.0, 200.0, 10.0, 10.0);
        let c = block(&mut s, 300.0, 300.0, 30.0, 30.0);
        s.execute("edit.group", &json!({"ids": [a, b]})).unwrap();
        let apart_before = bounds(&s, b).x0 - bounds(&s, a).x0;
        let result = s.execute("arrange.nest", &json!({"ids": [a, c], "rotations": 1})).unwrap();
        assert_eq!(result["placed"], 2, "the group counts once, and its other member came along: {result}");
        assert!((bounds(&s, b).x0 - bounds(&s, a).x0 - apart_before).abs() < 1e-6, "members keep their distance");
        assert!(apart(bounds(&s, a), bounds(&s, c), 2.0) && apart(bounds(&s, b), bounds(&s, c), 2.0));
        let whole = s.doc.bounds_of(&[a, b]).unwrap();
        assert!(whole.x0 >= -1e-6 && whole.y0 >= -1e-6, "{whole:?}");
    }

    #[test]
    fn parts_that_do_not_fit_stay_put_and_are_selected() {
        let mut s = Session::new();
        let small = block(&mut s, 5.0, 5.0, 10.0, 10.0);
        let huge = block(&mut s, 500.0, 500.0, 900.0, 900.0);
        let huge_before = bounds(&s, huge);
        let result = s.execute("arrange.nest", &json!({"ids": [small, huge]})).unwrap();
        assert_eq!((result["placed"].as_u64(), result["unplaced"].clone()), (Some(1), json!([huge])), "{result}");
        assert_eq!(bounds(&s, huge), huge_before, "the part that did not fit has not moved");
        assert_eq!(s.selection, vec![huge], "so the person can see what is left");
    }

    #[test]
    fn nesting_uses_the_region_asked_for_and_can_turn_parts() {
        let mut s = Session::new();
        let strip = block(&mut s, 0.0, 0.0, 60.0, 10.0);
        let region = json!({"ids": [strip], "x": 200, "y": 100, "width": 20, "height": 70});
        let mut fixed = Session::new();
        let other = block(&mut fixed, 0.0, 0.0, 60.0, 10.0);
        let no_turns = json!({"ids": [other], "x": 200, "y": 100, "width": 20, "height": 70, "rotations": 1});
        assert_eq!(fixed.execute("arrange.nest", &no_turns).unwrap()["placed"], 0, "60 mm cannot lie across a 20 mm region");
        assert_eq!(s.execute("arrange.nest", &region).unwrap()["placed"], 1);
        let b = bounds(&s, strip);
        assert!(b.x0 >= 200.0 - 1e-6 && b.x1 <= 220.0 + 1e-6 && b.y0 >= 100.0 - 1e-6 && b.y1 <= 170.0 + 1e-6, "{b:?}");
        assert!(b.height() > b.width(), "it was turned upright");
    }

    #[test]
    fn bad_nesting_requests_are_refused_and_change_nothing() {
        let mut s = Session::new();
        let a = block(&mut s, 30.0, 30.0, 10.0, 10.0);
        let before = bounds(&s, a);
        for bad in [
            json!({"gap": -1}),
            json!({"gap": 1e9}),
            json!({"margin": -5}),
            json!({"margin": 1e6}),
            json!({"rotations": 0}),
            json!({"rotations": 1000}),
            json!({"resolution": 0}),
            json!({"width": 0}),
            json!({"width": -10, "height": 10}),
            json!({"x": 1e300}),
            json!({"gap": "wide"}),
            json!({"ids": [999]}),
        ] {
            assert!(s.execute("arrange.nest", &bad).is_err(), "{bad}");
        }
        assert_eq!(bounds(&s, a), before);
        assert!(Session::new().execute("arrange.nest", &json!({})).is_err(), "nothing to nest");
    }
}

mod path_ops_tests {
    use super::*;

    fn two_overlapping(s: &mut Session) -> (u64, u64) {
        let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        (a, b)
    }

    fn bounds_of_selection(s: &Session) -> laserfoundry_geom::Rect {
        s.doc.bounds_of(&s.selection).unwrap()
    }

    #[test]
    fn union_replaces_the_inputs_with_one_shape_and_is_one_undo_step() {
        let mut s = Session::new();
        let (a, b) = two_overlapping(&mut s);
        let v = s.execute("path.boolean", &json!({"op": "union", "ids": [a, b]})).unwrap();
        assert_eq!(s.doc.shapes.len(), 1);
        let r = bounds_of_selection(&s);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (0.0, 0.0, 60.0, 60.0));
        assert_eq!(v["contours"], json!(1));
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.shapes.len(), 2);
    }

    #[test]
    fn difference_subtracts_the_later_shapes_and_can_keep_originals() {
        let mut s = Session::new();
        let (a, b) = two_overlapping(&mut s);
        s.execute("path.boolean", &json!({"op": "difference", "ids": [a, b], "keep_originals": true})).unwrap();
        assert_eq!(s.doc.shapes.len(), 3);
        let r = bounds_of_selection(&s);
        assert_eq!((r.x1, r.y1), (40.0, 40.0), "the L-shaped remainder stays within the first square");
    }

    #[test]
    fn intersection_of_disjoint_shapes_is_an_error_that_changes_nothing() {
        let mut s = Session::new();
        let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
        let before = s.doc.clone();
        assert!(s.execute("path.boolean", &json!({"op": "intersect", "ids": [a, b]})).is_err());
        assert_eq!(s.doc, before);
    }

    #[test]
    fn booleans_refuse_open_lines_images_and_single_shapes() {
        let mut s = Session::new();
        let a = s.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
        let line = s.execute("shape.line", &json!({"x1": 0, "y1": 0, "x2": 9, "y2": 9})).unwrap()["id"].as_u64().unwrap();
        assert!(s.execute("path.boolean", &json!({"op": "union", "ids": [a]})).is_err());
        let err = s.execute("path.boolean", &json!({"op": "union", "ids": [a, line]})).unwrap_err().to_string();
        assert!(err.contains("no closed outline"), "{err}");
    }

    #[test]
    fn a_rectangle_and_an_ellipse_can_be_welded_and_the_circle_becomes_a_hole_when_subtracted() {
        let mut s = Session::new();
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap();
        let e = s.execute("shape.ellipse", &json!({"cx": 30, "cy": 30, "rx": 10, "ry": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("path.boolean", &json!({"op": "difference", "ids": [r, e], "keep_originals": true})).unwrap();
        let result = s.doc.shape(*s.selection.first().unwrap()).unwrap();
        let closed = result.world_polylines().iter().filter(|l| l.closed).count();
        assert_eq!(closed, 2, "outer square plus the circular hole");
        // Welding works with the ellipse too (it used to be rejected as an open shape).
        assert!(s.execute("path.boolean", &json!({"op": "union", "ids": [r, e]})).is_ok());
    }

    #[test]
    fn offset_keeps_originals_by_default_and_grows_the_shape() {
        let mut s = Session::new();
        let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("path.offset", &json!({"ids": [a], "distance": 2, "join": "miter"})).unwrap();
        assert_eq!(s.doc.shapes.len(), 2);
        let r = bounds_of_selection(&s);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (8.0, 8.0, 32.0, 32.0));
        s.execute("path.offset", &json!({"distance": -2, "join": "miter", "keep_originals": false})).unwrap();
        assert_eq!(s.doc.shapes.len(), 2, "the offset shape was replaced by its shrunken copy");
        let r = bounds_of_selection(&s);
        assert_eq!((r.x0, r.x1), (10.0, 30.0));
    }

    #[test]
    fn offset_of_open_text_outlines_the_strokes_and_collapse_is_reported() {
        let mut s = Session::new();
        s.execute("text.create", &json!({"text": "I", "height": 10})).unwrap();
        s.execute("path.offset", &json!({"distance": 0.5})).unwrap();
        assert_eq!(s.doc.shapes.len(), 2);
        let tiny = s.execute("shape.rectangle", &json!({"width": 2, "height": 2})).unwrap()["id"].as_u64().unwrap();
        assert!(s.execute("path.offset", &json!({"ids": [tiny], "distance": -5})).is_err(), "collapsed");
        assert!(s.execute("path.offset", &json!({"ids": [tiny], "distance": -0.5})).is_ok());
    }

    #[test]
    fn offset_validates_its_distance() {
        let mut s = Session::new();
        let a = s.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
        for d in [json!(1e9), json!("x"), json!(null)] {
            assert!(s.execute("path.offset", &json!({"ids": [a], "distance": d})).is_err());
        }
        assert!(s.execute("path.offset", &json!({"distance": 1})).is_ok(), "uses the selection");
    }
}

mod text_tests {
    use super::*;

    #[test]
    fn text_is_created_placed_and_engraved_as_lines() {
        let mut s = Session::new();
        s.execute("machine.set", &json!({"origin": "top_left"})).unwrap();
        let v = s.execute("text.create", &json!({"text": "HELLO", "x": 20, "y": 30, "height": 12})).unwrap();
        let id = v["id"].as_u64().unwrap();
        let b = s.doc.bounds_of(&[id]).unwrap();
        assert!((b.height() - 12.0).abs() < 1e-6 && (b.x0 - 20.0).abs() < 1e-6 && (b.y0 - 30.0).abs() < 1e-6, "{b:?}");
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
        assert!(g["gcode"].as_str().unwrap().matches("G1").count() > 50, "text becomes many cutting moves");
    }

    #[test]
    fn editing_text_regenerates_the_geometry_in_one_undo_step() {
        let mut s = Session::new();
        let id = s.execute("text.create", &json!({"text": "AB", "height": 10})).unwrap()["id"].as_u64().unwrap();
        let narrow = s.doc.bounds_of(&[id]).unwrap().width();
        s.execute("text.edit", &json!({"text": "ABCDEFG", "height": 20})).unwrap();
        let wide = s.doc.bounds_of(&[id]).unwrap();
        assert!(wide.width() > narrow * 4.0 && (wide.height() - 20.0).abs() < 1e-6, "{wide:?}");
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!((s.doc.bounds_of(&[id]).unwrap().width() - narrow).abs() < 1e-9);
    }

    #[test]
    fn a_moved_text_keeps_its_transform_when_edited() {
        let mut s = Session::new();
        let id = s.execute("text.create", &json!({"text": "HI", "x": 0, "y": 0})).unwrap()["id"].as_u64().unwrap();
        s.execute("edit.move", &json!({"dx": 40, "dy": 25})).unwrap();
        s.execute("text.edit", &json!({"text": "HELLO"})).unwrap();
        let b = s.doc.bounds_of(&[id]).unwrap();
        assert!((b.x0 - 40.0).abs() < 1e-6 && (b.y0 - 25.0).abs() < 1e-6, "{b:?}");
    }

    #[test]
    fn bad_text_input_is_an_error_and_unsupported_characters_are_reported() {
        let mut s = Session::new();
        assert!(s.execute("text.create", &json!({"text": "   "})).is_err());
        assert!(s.execute("text.create", &json!({"text": "A", "height": -3})).is_err());
        assert!(s.execute("text.create", &json!({"text": "A", "height": 1e12})).is_err());
        assert!(s.execute("text.edit", &json!({})).is_err(), "nothing selected");
        let v = s.execute("text.create", &json!({"text": "caf\u{e9}"})).unwrap();
        assert_eq!(v["unsupported"], json!(["\u{e9}"]));
        let rect = s.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
        assert!(s.execute("text.edit", &json!({"id": rect, "text": "x"})).is_err(), "only text can be edited as text");
    }

    #[test]
    fn text_survives_save_and_load() {
        let mut s = Session::new();
        s.execute("text.create", &json!({"text": "SAVE ME", "align": "center"})).unwrap();
        let back = laserfoundry_format::from_bytes(&laserfoundry_format::to_bytes(&s.doc).unwrap()).unwrap();
        assert_eq!(back.shapes, s.doc.shapes);
    }
}

mod import_tests {
    use super::*;

    #[test]
    fn svg_colours_map_to_layers() {
        let mut s = Session::new();
        s.execute("file.import_svg", &json!({"data": SVG})).unwrap();
        let layers: Vec<_> = s.doc.shapes.iter().map(|sh| sh.layer).collect();
        assert_eq!(layers, vec![2, 1]); // #ff0000 -> C02, #0000ff -> C01
    }

    #[test]
    fn dxf_layers_become_cut_layers() {
        let dxf =
            "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nA\n10\n0\n20\n0\n11\n10\n21\n0\n0\nLINE\n8\nB\n10\n0\n20\n5\n11\n10\n21\n5\n0\nENDSEC\n0\nEOF\n";
        let mut s = Session::new();
        s.execute("file.import_dxf", &json!({"data": dxf})).unwrap();
        let layers: Vec<_> = s.doc.shapes.iter().map(|sh| sh.layer).collect();
        assert_eq!(layers, vec![0, 1]);
    }

    #[test]
    fn four_loose_dxf_lines_import_as_one_closed_fillable_shape() {
        let line = |x1: f64, y1: f64, x2: f64, y2: f64| format!("0\nLINE\n8\nCUT\n10\n{x1}\n20\n{y1}\n11\n{x2}\n21\n{y2}\n");
        let dxf = format!(
            "0\nSECTION\n2\nENTITIES\n{}{}{}{}0\nENDSEC\n0\nEOF\n",
            line(0.0, 0.0, 20.0, 0.0),
            line(20.0, 0.0, 20.0, 10.0),
            line(20.0, 10.0, 0.0, 10.0),
            line(0.0, 10.0, 0.0, 0.0)
        );
        let mut s = Session::new();
        s.execute("file.import_dxf", &json!({"data": dxf})).unwrap();
        assert_eq!(s.doc.shapes.len(), 1, "joined into one shape");
        assert!(s.doc.shapes[0].world_polylines().iter().all(|l| l.closed));
        // A closed shape fills; four open lines would not have.
        s.execute("layer.set", &json!({"id": 0, "mode": "fill", "interval": 1})).unwrap();
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
        assert!(g["gcode"].as_str().unwrap().matches("G1").count() >= 10);
        // join: false keeps the segments separate.
        let mut t = Session::new();
        t.execute("file.import_dxf", &json!({"data": dxf, "join": false})).unwrap();
        assert_eq!(t.doc.shapes.len(), 4);
        // And joining afterwards by hand gets the same result.
        t.execute("edit.select_all", &Value::Null).unwrap();
        let v = t.execute("path.join", &json!({})).unwrap();
        assert_eq!((v["closed"].as_u64(), v["open"].as_u64()), (Some(1), Some(0)));
        assert_eq!(t.doc.shapes.len(), 1);
    }

    #[test]
    fn joining_keeps_shapes_on_their_own_layers_and_validates() {
        let mut s = Session::new();
        let a = s.execute("shape.line", &json!({"x1": 0, "y1": 0, "x2": 10, "y2": 0, "layer": 1})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("shape.line", &json!({"x1": 10, "y1": 0, "x2": 10, "y2": 10, "layer": 1})).unwrap()["id"].as_u64().unwrap();
        let c = s.execute("shape.line", &json!({"x1": 10, "y1": 10, "x2": 20, "y2": 10, "layer": 2})).unwrap()["id"].as_u64().unwrap();
        s.execute("path.join", &json!({"ids": [a, b, c]})).unwrap();
        let mut layers: Vec<_> = s.doc.shapes.iter().map(|sh| (sh.layer, sh.world_polylines()[0].points.len())).collect();
        layers.sort();
        assert_eq!(layers, vec![(1, 3), (2, 2)], "the layer-2 line is not merged into the layer-1 chain");
        assert!(s.execute("path.join", &json!({"tolerance": -1})).is_err());
        assert!(s.execute("path.join", &json!({"tolerance": "x"})).is_err());
        let before = s.doc.clone();
        assert!(s.execute("path.join", &json!({"ids": []})).is_err());
        assert_eq!(s.doc, before);
    }

    #[test]
    fn imports_are_undoable_as_one_step() {
        let mut s = Session::new();
        s.execute("file.import_svg", &json!({"data": SVG})).unwrap();
        assert_eq!(s.doc.shapes.len(), 2);
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty());
    }

    #[test]
    fn bad_input_is_an_error_not_a_crash() {
        let mut s = Session::new();
        assert!(s.execute("file.import_svg", &json!({"data": "<svg"})).is_err());
        assert!(s.execute("file.import_dxf", &json!({"data": "junk"})).is_err());
        assert!(s.execute("file.import_svg", &json!({})).is_err());
        assert!(s.execute("file.import_image", &json!({"path": "/definitely/not/here.png"})).is_err());
    }

    #[test]
    fn presets_are_saved_applied_replaced_deleted_and_persist_with_the_project() {
        let mut s = Session::new();
        s.execute("layer.set", &json!({"id": 0, "speed": 450, "max_power": 85, "passes": 2, "name": "Cut ply"})).unwrap();
        s.execute("preset.save", &json!({"name": "  Birch 3 mm  ", "layer": 0})).unwrap();
        s.execute("layer.set", &json!({"id": 1, "name": "Mark"})).unwrap();
        s.execute("preset.apply", &json!({"name": "Birch 3 mm", "layer": 1})).unwrap();
        let l = s.doc.layer(1).unwrap();
        assert_eq!((l.speed, l.max_power, l.passes, l.name.as_str()), (450.0, 85.0, 2, "Mark"), "settings copied, name kept");

        s.execute("layer.set", &json!({"id": 0, "speed": 100})).unwrap();
        let v = s.execute("preset.save", &json!({"name": "Birch 3 mm", "layer": 0})).unwrap();
        assert_eq!(v["count"], json!(1), "same name replaces");
        let back = laserfoundry_format::from_bytes(&laserfoundry_format::to_bytes(&s.doc).unwrap()).unwrap();
        assert_eq!(back.presets.len(), 1);
        assert_eq!(back.presets[0].settings.speed, 100.0);

        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.presets[0].settings.speed, 450.0, "saving is one undo step");
        for bad in [
            ("preset.apply", json!({"name": "Nope", "layer": 0})),
            ("preset.apply", json!({"name": "Birch 3 mm", "layer": 999})),
            ("preset.save", json!({"name": "   ", "layer": 0})),
            ("preset.save", json!({"name": "x", "layer": 999})),
            ("preset.delete", json!({"name": "Nope"})),
        ] {
            assert!(s.execute(bad.0, &bad.1).is_err(), "{bad:?}");
        }
        s.execute("preset.delete", &json!({"name": "Birch 3 mm"})).unwrap();
        assert!(s.doc.presets.is_empty());
    }

    #[test]
    fn starter_presets_are_added_once_and_apply_like_any_other() {
        let mut s = Session::new();
        let v = s.execute("preset.add_starters", &Value::Null).unwrap();
        let n = v["added"].as_u64().unwrap();
        assert!(n >= 5 && s.doc.presets.len() as u64 == n);
        assert_eq!(s.execute("preset.add_starters", &Value::Null).unwrap()["added"], json!(0), "never twice");
        s.execute("preset.delete", &json!({"name": "Starter: thin cut"})).unwrap();
        assert_eq!(s.execute("preset.add_starters", &Value::Null).unwrap()["added"], json!(1), "only what is missing comes back");
        s.execute("preset.apply", &json!({"name": "Starter: thin cut", "layer": 3})).unwrap();
        let l = s.doc.layer(3).unwrap();
        assert_eq!((l.mode, l.speed, l.max_power), (laserfoundry_doc::LayerMode::Line, 600.0, 90.0));
        for p in &s.doc.presets {
            let l = &p.settings;
            assert!(
                l.speed > 0.0 && (0.0..=100.0).contains(&l.max_power) && l.min_power <= l.max_power && l.interval > 0.0 && l.passes >= 1,
                "{}",
                p.name
            );
        }
    }

    #[test]
    fn presets_travel_between_projects_through_export_and_import() {
        let mut a = Session::new();
        a.execute("layer.set", &json!({"id": 0, "speed": 321, "max_power": 44})).unwrap();
        a.execute("preset.save", &json!({"name": "Mine", "layer": 0})).unwrap();
        let exported = a.execute("preset.export", &Value::Null).unwrap();
        let mut b = Session::new();
        let v = b.execute("preset.import", &exported).unwrap();
        assert_eq!(v["added"], json!(1));
        assert_eq!(b.doc.presets[0].settings.speed, 321.0);
        assert_eq!(b.execute("preset.import", &exported).unwrap()["added"], json!(0), "names already present are kept");
        b.execute("edit.undo", &Value::Null).unwrap();
        b.execute("edit.undo", &Value::Null).unwrap();
        assert!(b.doc.presets.is_empty());

        let mut bad = exported.clone();
        bad["presets"][0]["settings"]["speed"] = json!(0);
        assert!(b.execute("preset.import", &bad).is_err(), "unusable settings are refused");
        bad["presets"][0]["name"] = json!("  ");
        assert!(b.execute("preset.import", &bad).is_err());
        assert!(b.execute("preset.import", &json!({"presets": "x"})).is_err());
        assert!(b.execute("preset.import", &json!({"presets": [{"name": "x"}]})).is_err());
    }

    #[test]
    fn the_number_of_presets_is_capped() {
        let mut s = Session::new();
        for i in 0..laserfoundry_doc::MAX_PRESETS {
            s.execute("preset.save", &json!({"name": format!("p{i}"), "layer": 0})).unwrap();
        }
        assert!(s.execute("preset.save", &json!({"name": "one too many", "layer": 0})).is_err());
        assert!(s.execute("preset.save", &json!({"name": "p0", "layer": 0})).is_ok(), "replacing is still allowed at the cap");
    }

    #[test]
    fn perforation_is_a_layer_setting_that_adds_rapid_gaps_to_the_program() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
        let rapids = |s: &mut Session| {
            s.execute("job.gcode", &json!({"include": true})).unwrap()["gcode"].as_str().unwrap().lines().filter(|l| l.starts_with("G0")).count()
        };
        let solid = rapids(&mut s);
        s.execute("layer.set", &json!({"id": 0, "perf_cut": 5, "perf_gap": 5})).unwrap();
        assert!(rapids(&mut s) > solid + 10);
        assert!(s.execute("layer.set", &json!({"id": 0, "perf_cut": -1})).is_err());
        assert!(s.execute("layer.set", &json!({"id": 0, "perf_gap": 5000})).is_err());
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.layer(0).unwrap().perf_cut, 0.0);
    }

    #[test]
    fn the_gcode_dialect_is_a_machine_setting_that_changes_the_program() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
        let program = |s: &mut Session| s.execute("job.gcode", &json!({"include": true})).unwrap()["gcode"].as_str().unwrap().to_string();
        assert!(program(&mut s).contains("M4 S0"));
        s.execute("machine.set", &json!({"dialect": "marlin"})).unwrap();
        assert!(program(&mut s).contains("M3 I S0"));
        s.execute("machine.set", &json!({"dialect": "smoothie"})).unwrap();
        assert!(!program(&mut s).contains("M4"));
        assert!(s.execute("machine.set", &json!({"dialect": "ruida"})).is_err());
    }

    #[test]
    fn svg_export_writes_millimetre_layers_that_import_back_to_the_same_size() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 30, "height": 15})).unwrap();
        s.execute("shape.ellipse", &json!({"cx": 80, "cy": 40, "rx": 10, "ry": 5, "layer": 1})).unwrap();
        s.execute("layer.set", &json!({"id": 1, "mode": "fill", "name": "Fill \"A\" <1>"})).unwrap();
        s.execute("text.create", &json!({"text": "Hi", "x": 10, "y": 60, "height": 8, "layer": 2})).unwrap();
        let v = s.execute("file.export_svg", &json!({"include": true})).unwrap();
        let svg = v["svg"].as_str().unwrap();
        assert_eq!(v["shapes"], json!(3));
        assert!(svg.contains("width=\"") && svg.contains("mm\"") && svg.contains("viewBox=\"10 20 80 48\""), "{svg}");
        assert!(svg.contains("id=\"layer-0\"") && svg.contains("id=\"layer-1\"") && svg.contains("fill-rule=\"evenodd\""));
        assert!(svg.contains("data-name=\"Fill &quot;A&quot; &lt;1&gt;\""), "names are escaped");
        assert!(!svg.contains("layer-3"), "empty layers are left out");

        // It reads back at the same size and place.
        let before = s
            .doc
            .shapes
            .iter()
            .filter_map(|sh| sh.world_bounds())
            .fold(None, |a: Option<laserfoundry_geom::Rect>, b| Some(a.map_or(b, |a| a.union(b))))
            .unwrap();
        let mut t = Session::new();
        t.execute("file.import_svg", &json!({"data": svg})).unwrap();
        let after = t
            .doc
            .shapes
            .iter()
            .filter_map(|sh| sh.world_bounds())
            .fold(None, |a: Option<laserfoundry_geom::Rect>, b| Some(a.map_or(b, |a| a.union(b))))
            .unwrap();
        assert!((before.width() - after.width()).abs() < 0.5 && (before.height() - after.height()).abs() < 0.5, "{before:?} vs {after:?}");
    }

    #[test]
    fn svg_export_can_limit_to_the_selection_include_hidden_layers_and_refuses_nothing() {
        let mut s = Session::new();
        assert!(s.execute("file.export_svg", &json!({"include": true})).is_err(), "nothing to export");
        let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 10, "height": 10, "layer": 4})).unwrap();
        s.selection = vec![a];
        assert_eq!(s.execute("file.export_svg", &json!({"include": true, "selection_only": true})).unwrap()["shapes"], json!(1));
        s.execute("layer.set", &json!({"id": 4, "output": false})).unwrap();
        assert_eq!(s.execute("file.export_svg", &json!({"include": true})).unwrap()["shapes"], json!(1), "layers with output off are skipped");
        assert_eq!(s.execute("file.export_svg", &json!({"include": true, "include_hidden": true})).unwrap()["shapes"], json!(2));
        assert!(s.execute("file.export_svg", &json!({})).is_err(), "needs a path or include");
        let dir = std::env::temp_dir().join(format!("lf-engine-svgout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("out.svg");
        s.execute("file.export_svg", &json!({"path": file})).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().starts_with("<?xml"));
        assert!(s.execute("file.export_svg", &json!({"path": dir.join("no/such/dir/x.svg")})).is_err());
    }

    #[test]
    fn svg_export_embeds_pictures_as_png_with_their_placement() {
        let dir = std::env::temp_dir().join(format!("lf-engine-svgimg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.png");
        image::GrayImage::from_fn(4, 2, |x, _| image::Luma([(x * 60) as u8])).save(&path).unwrap();
        let mut s = Session::new();
        s.execute("file.import_image", &json!({"path": path, "width": 40, "x": 5, "y": 7})).unwrap();
        let svg = s.execute("file.export_svg", &json!({"include": true})).unwrap()["svg"].as_str().unwrap().to_string();
        assert!(svg.contains("<image ") && svg.contains("href=\"data:image/png;base64,") && svg.contains("matrix("), "{svg}");
    }

    #[test]
    fn tracing_by_colour_makes_a_shape_per_palette_colour_and_needs_a_coloured_picture() {
        let dir = std::env::temp_dir().join(format!("lf-engine-colours-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (colour, grey) = (dir.join("c.png"), dir.join("g.png"));
        // White 20x10 with a red block (columns 2..8) and a blue block (columns 12..18).
        image::RgbImage::from_fn(20, 10, |x, _| {
            image::Rgb(if (2..8).contains(&x) {
                [220, 20, 20]
            } else if (12..18).contains(&x) {
                [20, 20, 220]
            } else {
                [255, 255, 255]
            })
        })
        .save(&colour)
        .unwrap();
        image::GrayImage::from_fn(20, 10, |x, _| image::Luma([if x < 10 { 0 } else { 255 }])).save(&grey).unwrap();
        let mut s = Session::new();
        let pic = s.execute("file.import_image", &json!({"path": colour, "width": 40})).unwrap()["id"].as_u64().unwrap();
        let v = s.execute("image.trace", &json!({"id": pic, "colors": 2})).unwrap();
        let ids = v["ids"].as_array().unwrap();
        assert_eq!((ids.len(), v["colors"].as_u64()), (2, Some(2)), "{v}");
        let layers: Vec<u32> = ids.iter().map(|i| s.doc.shape(i.as_u64().unwrap()).unwrap().layer).collect();
        assert_ne!(layers[0], layers[1], "red and blue land on different layers");
        for i in ids {
            let b = s.doc.shape(i.as_u64().unwrap()).unwrap().world_bounds().unwrap();
            assert!((b.x1 - b.x0 - 12.0).abs() < 1e-6, "each block is 6 px = 12 mm wide: {b:?}");
        }
        assert!(s.execute("image.trace", &json!({"id": pic, "colors": 1})).is_err());
        assert!(s.execute("image.trace", &json!({"id": pic, "colors": 99})).is_err());
        // The colour survives crop and is saved in the project.
        s.execute("image.crop", &json!({"id": pic, "x": 0, "y": 0, "width": 10, "height": 10})).ok();
        let back = laserfoundry_format::from_bytes(&laserfoundry_format::to_bytes(&s.doc).unwrap()).unwrap();
        assert!(matches!(&back.shape(pic).unwrap().kind, laserfoundry_doc::ShapeKind::Image { image, .. } if image.rgb.is_some()));

        let grey_pic = s.execute("file.import_image", &json!({"path": grey, "width": 40})).unwrap()["id"].as_u64().unwrap();
        assert!(s.execute("image.trace", &json!({"id": grey_pic, "colors": 2})).is_err(), "a grey picture has no colours");
    }

    #[test]
    fn tracing_with_curve_fitting_needs_far_fewer_nodes_than_smoothing_and_validates() {
        let dir = std::env::temp_dir().join(format!("lf-engine-fit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("disc.png");
        // A black disc of radius 40 on white.
        image::GrayImage::from_fn(100, 100, |x, y| {
            let (dx, dy) = (f64::from(x) - 50.0, f64::from(y) - 50.0);
            image::Luma([if dx * dx + dy * dy < 1600.0 { 0 } else { 255 }])
        })
        .save(&path)
        .unwrap();
        let mut s = Session::new();
        let pic = s.execute("file.import_image", &json!({"path": path, "width": 100})).unwrap()["id"].as_u64().unwrap();
        let nodes = |s: &Session, id: u64| match &s.doc.shape(id).unwrap().kind {
            laserfoundry_doc::ShapeKind::Path { path } => path.elements().len(),
            _ => 0,
        };
        let smooth = s.execute("image.trace", &json!({"id": pic, "smooth": true, "tolerance": 1.0})).unwrap()["id"].as_u64().unwrap();
        let fitted = s.execute("image.trace", &json!({"id": pic, "fit": 0.5, "tolerance": 1.0})).unwrap()["id"].as_u64().unwrap();
        let (a, b) = (nodes(&s, smooth), nodes(&s, fitted));
        assert!(b * 2 < a, "fitted {b} elements against smoothed {a}");
        let bounds = s.doc.shape(fitted).unwrap().world_bounds().unwrap();
        assert!((bounds.x1 - bounds.x0 - 80.0).abs() < 2.0, "{bounds:?}");
        for bad in [json!({"id": pic, "fit": 0}), json!({"id": pic, "fit": 100}), json!({"id": pic, "fit": "x"})] {
            assert!(s.execute("image.trace", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn tracing_in_tonal_bands_makes_nested_outlines_from_light_to_dark() {
        let dir = std::env::temp_dir().join(format!("lf-engine-bands-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.png");
        // Mid-grey 12x8 block with a black 4x4 core, on white.
        image::GrayImage::from_fn(20, 12, |x, y| {
            let core = (8..12).contains(&x) && (4..8).contains(&y);
            let block = (4..16).contains(&x) && (2..10).contains(&y);
            image::Luma([if core {
                0
            } else if block {
                100
            } else {
                255
            }])
        })
        .save(&path)
        .unwrap();
        let mut s = Session::new();
        let pic = s.execute("file.import_image", &json!({"path": path, "width": 20})).unwrap()["id"].as_u64().unwrap();
        let v = s.execute("image.trace", &json!({"id": pic, "threshold": 128, "levels": 2})).unwrap();
        let ids = v["ids"].as_array().unwrap();
        assert_eq!(ids.len(), 2, "{v}");
        let width = |i: &Value| {
            let b = s.doc.shape(i.as_u64().unwrap()).unwrap().world_bounds().unwrap();
            b.x1 - b.x0
        };
        // Threshold 64 catches only the core; 128 the whole block.
        assert!((width(&ids[0]) - 4.0).abs() < 1e-6 && (width(&ids[1]) - 12.0).abs() < 1e-6, "{} {}", width(&ids[0]), width(&ids[1]));
        assert_eq!(s.selection.len(), 2);
        for bad in [json!({"id": pic, "levels": 0}), json!({"id": pic, "levels": 9})] {
            assert!(s.execute("image.trace", &bad).is_err(), "{bad}");
        }
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.shapes.len(), 1, "both bands go in one undo step");
    }

    #[test]
    fn tracing_a_picture_adds_outlines_in_place_and_validates_its_inputs() {
        let dir = std::env::temp_dir().join(format!("lf-engine-trace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.png");
        // 20x10 white picture with a black 6x4 block at columns 4..10, rows 3..7.
        image::GrayImage::from_fn(20, 10, |x, y| image::Luma([if (4..10).contains(&x) && (3..7).contains(&y) { 0 } else { 255 }]))
            .save(&path)
            .unwrap();
        let mut s = Session::new();
        let pic = s.execute("file.import_image", &json!({"path": path, "width": 40, "x": 100, "y": 50})).unwrap()["id"].as_u64().unwrap();
        let v = s.execute("image.trace", &json!({"id": pic})).unwrap();
        assert_eq!(v["outlines"], json!(1));
        let traced = v["id"].as_u64().unwrap();
        let b = s.doc.shape(traced).unwrap().world_bounds().unwrap();
        // 2 mm per pixel: the block spans x 100+8..100+20 and y 50+6..50+14.
        assert!((b.x0 - 108.0).abs() < 1e-6 && (b.x1 - 120.0).abs() < 1e-6 && (b.y0 - 56.0).abs() < 1e-6 && (b.y1 - 64.0).abs() < 1e-6, "{b:?}");
        assert_eq!(s.selection, vec![traced]);
        assert_eq!(s.doc.shapes.len(), 2, "the picture is kept by default");

        s.execute("image.trace", &json!({"id": pic, "keep_image": false})).unwrap();
        assert!(s.doc.shape(pic).is_none());
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shape(pic).is_some(), "one undo step brings the picture back");

        s.execute("image.adjust", &json!({"id": pic, "negative": true})).unwrap();
        let negative = s.execute("image.trace", &json!({"id": pic})).unwrap();
        let nb = s.doc.shape(negative["id"].as_u64().unwrap()).unwrap().world_bounds().unwrap();
        assert!(nb.x1 - nb.x0 > 30.0, "the negative traces everything but the block: {nb:?}");
        s.execute("image.adjust", &json!({"id": pic, "reset": true})).unwrap();

        for bad in [
            json!({"id": pic, "threshold": 300}),
            json!({"id": pic, "tolerance": -1}),
            json!({"id": pic, "min_area": "x"}),
            json!({"id": pic, "layer": 999}),
        ] {
            assert!(s.execute("image.trace", &bad).is_err(), "{bad}");
        }
        assert!(s.execute("image.trace", &json!({"id": pic, "threshold": 0})).is_err(), "nothing is darker than 0");
        s.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap();
        assert!(s.execute("image.trace", &json!({})).is_err(), "a rectangle is not an image");
    }

    #[test]
    fn cutting_only_the_selection_limits_gcode_estimate_and_frame() {
        let mut s = Session::new();
        let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("shape.rectangle", &json!({"x": 200, "y": 100, "width": 40, "height": 30})).unwrap();
        let frame = |s: &mut Session| s.execute("job.frame", &json!({})).unwrap()["bounds"].clone();
        assert_eq!(frame(&mut s), json!([10.0, 10.0, 240.0, 130.0]));
        let all_time = s.execute("job.estimate", &Value::Null).unwrap()["seconds"].as_f64().unwrap();

        s.execute("edit.select", &json!({"ids": [a]})).unwrap();
        s.execute("job.set_scope", &json!({"selection_only": true})).unwrap();
        assert_eq!(frame(&mut s), json!([10.0, 10.0, 30.0, 20.0]));
        assert!(s.execute("job.estimate", &Value::Null).unwrap()["seconds"].as_f64().unwrap() < all_time);
        let gcode = s.execute("job.gcode", &json!({"include": true})).unwrap()["gcode"].as_str().unwrap().to_string();
        assert!(!gcode.contains("X200") && !gcode.contains("X240"), "{gcode}");

        s.execute("edit.select", &json!({"ids": []})).unwrap();
        assert!(s.execute("job.frame", &json!({})).is_err(), "nothing selected, nothing to frame");
        assert_eq!(s.doc.shapes.len(), 2, "the document itself is untouched");
        s.execute("job.set_scope", &json!({"selection_only": false})).unwrap();
        assert_eq!(frame(&mut s), json!([10.0, 10.0, 240.0, 130.0]));
        assert!(s.execute("job.set_scope", &json!({"selection_only": "yes"})).is_err());
    }

    #[test]
    fn distributing_evens_out_the_gaps_and_stars_have_alternating_radii() {
        let mut s = Session::new();
        let mut ids = Vec::new();
        for (x, w) in [(0.0, 10.0), (15.0, 20.0), (90.0, 10.0), (40.0, 5.0)] {
            ids.push(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": w, "height": 5})).unwrap()["id"].as_u64().unwrap());
        }
        let v = s.execute("edit.distribute", &json!({"ids": ids, "axis": "horizontal"})).unwrap();
        // Widths 10+20+10+5 = 45 inside a 100 mm span: three equal gaps of 55/3.
        assert!((v["gap"].as_f64().unwrap() - 55.0 / 3.0).abs() < 1e-9, "{v}");
        let mut spans: Vec<(f64, f64)> = ids.iter().map(|id| s.doc.shape(*id).unwrap().world_bounds().unwrap()).map(|b| (b.x0, b.x1)).collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        for pair in spans.windows(2) {
            assert!((pair[1].0 - pair[0].1 - 55.0 / 3.0).abs() < 1e-9, "{spans:?}");
        }
        assert!(spans[0].0.abs() < 1e-9 && (spans[3].1 - 100.0).abs() < 1e-9, "the outermost shapes stay put: {spans:?}");

        s.execute("edit.distribute", &json!({"ids": ids, "axis": "vertical", "gap": 2})).unwrap();
        let top = s.doc.shape(ids[0]).unwrap().world_bounds().unwrap();
        assert!(top.y0.abs() < 1e-9);
        assert!(s.execute("edit.distribute", &json!({"ids": [ids[0], ids[1]], "axis": "horizontal"})).is_err(), "two shapes need a gap");
        assert!(s.execute("edit.distribute", &json!({"ids": ids, "axis": "horizontal", "gap": 1e9})).is_err());
        assert!(s.execute("edit.distribute", &json!({"ids": ids, "axis": "diagonal"})).is_err());

        let star = s.execute("shape.star", &json!({"cx": 50, "cy": 50, "outer_radius": 20, "inner_radius": 8, "points": 5})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let b = s.doc.shape(star).unwrap().world_bounds().unwrap();
        assert!((b.y0 - 30.0).abs() < 1e-9, "one tip points straight up: {b:?}");
        for bad in [
            json!({"cx": 0, "cy": 0, "outer_radius": 5, "inner_radius": 8, "points": 5}),
            json!({"cx": 0, "cy": 0, "outer_radius": 5, "inner_radius": 0, "points": 5}),
            json!({"cx": 0, "cy": 0, "outer_radius": 5, "inner_radius": 2, "points": 2}),
            json!({"cx": f64::NAN, "cy": 0, "outer_radius": 5, "inner_radius": 2, "points": 5}),
        ] {
            assert!(s.execute("shape.star", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_job_can_be_resumed_from_a_line_and_the_rest_matches_the_original() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
        let full = s.execute("job.gcode", &json!({"include": true})).unwrap()["gcode"].as_str().unwrap().to_string();
        let total = full.lines().count();
        let mid = total / 2;
        let r = s.execute("job.resume", &json!({"line": mid, "include": true})).unwrap();
        let text = r["gcode"].as_str().unwrap();
        assert_eq!(r["summary"]["total_lines"], json!(total));
        assert!(text.starts_with("; LaserFoundry resume"), "{text}");
        assert!(text.contains("\nG0 X"), "it first moves to where the head would be: {text}");
        assert!(text.trim_end().ends_with(full.lines().last().unwrap()), "same shutdown");
        // The resumed program still passes the streamer and the simulator.
        s.execute("job.simulate", &Value::Null).unwrap();
        assert!(s.execute("job.resume", &json!({"line": 0})).is_err());
        assert!(s.execute("job.resume", &json!({"line": total + 1})).is_err());
        assert!(s.execute("job.resume", &json!({"line": "x"})).is_err());
    }

    #[test]
    fn z_and_rotary_settings_are_validated_undoable_and_reach_the_program() {
        let mut s = Session::new();
        s.execute("machine.set", &json!({"origin": "top_left"})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 40})).unwrap();
        let gcode = |s: &mut Session| s.execute("job.gcode", &json!({"include": true})).unwrap()["gcode"].as_str().unwrap().to_string();
        assert!(!gcode(&mut s).contains(" Z"));
        s.execute("layer.set", &json!({"id": 0, "z_offset": -3, "z_per_pass": -1, "passes": 2})).unwrap();
        let g = gcode(&mut s);
        assert!(g.contains("G0 Z-3") && g.contains("G0 Z-4"), "{g}");
        assert!(s.execute("layer.set", &json!({"id": 0, "z_offset": 999})).is_err());
        s.execute("machine.set", &json!({"rotary_enabled": true, "rotary_reference_diameter": 80, "rotary_object_diameter": 40})).unwrap();
        assert!(gcode(&mut s).contains("Y100"), "50 mm of surface becomes 100 mm of Y");
        for bad in [json!({"rotary_object_diameter": 0}), json!({"rotary_reference_diameter": -5}), json!({"rotary_object_diameter": 1e9})] {
            assert!(s.execute("machine.set", &bad).is_err(), "{bad}");
        }
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(!s.doc.machine.rotary.enabled);
    }

    #[test]
    fn a_fallback_font_supplies_characters_the_main_font_lacks() {
        let dir = std::env::temp_dir().join(format!("lf-engine-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (main, other) = (dir.join("main.ttf"), dir.join("other.ttf"));
        std::fs::write(&main, laserfoundry_text::testfont::tiny_font()).unwrap();
        std::fs::write(&other, laserfoundry_text::testfont::tiny_font_for_cd()).unwrap();
        let mut s = Session::new();
        let v = s.execute("text.create", &json!({"text": "AC", "font": main})).unwrap();
        assert_eq!(v["unsupported"], json!(["C"]));
        let id = v["id"].as_u64().unwrap();
        let v = s.execute("text.edit", &json!({"id": id, "fallback_font": other})).unwrap();
        assert!(v["unsupported"].as_array().unwrap().is_empty(), "{v}");
        assert!(s.execute("text.edit", &json!({"id": id, "fallback_font": "/no/such.ttf"})).is_err());
        let v = s.execute("text.edit", &json!({"id": id, "fallback_font": ""})).unwrap();
        assert_eq!(v["unsupported"], json!(["C"]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn outline_text_from_a_font_file_is_filled_survives_without_the_font_and_can_switch_back() {
        let dir = std::env::temp_dir().join(format!("lf-engine-font-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let font = dir.join("tiny.ttf");
        std::fs::write(&font, laserfoundry_text::testfont::tiny_font()).unwrap();
        let mut s = Session::new();
        let v = s.execute("text.create", &json!({"text": "AB", "x": 20, "y": 30, "height": 14, "font": font})).unwrap();
        let id = v["id"].as_u64().unwrap();
        let b = s.doc.shape(id).unwrap().world_bounds().unwrap();
        assert!((b.y1 - b.y0 - 14.0).abs() < 0.1 && b.x0 >= 19.9, "{b:?}");

        // Fill mode engraves it solid, with the hole in the A left alone.
        s.execute("layer.set", &json!({"id": 0, "mode": "fill", "interval": 0.5})).unwrap();
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap()["gcode"].as_str().unwrap().to_string();
        assert!(g.lines().filter(|l| l.starts_with("G1")).count() > 40, "many scan lines");

        // The project keeps the shapes; opening it needs no font.
        std::fs::remove_file(&font).unwrap();
        let back = laserfoundry_format::from_bytes(&laserfoundry_format::to_bytes(&s.doc).unwrap()).unwrap();
        assert!(back.shape(id).unwrap().world_bounds().is_some());
        assert!(s.execute("text.edit", &json!({"id": id, "text": "BA"})).is_err(), "editing the text needs the font again");
        let r = s.execute("text.edit", &json!({"id": id, "text": "BA", "font": ""})).unwrap();
        assert!(r["unsupported"].as_array().unwrap().is_empty());
        assert!(s.execute("text.create", &json!({"text": "A", "font": "/no/such/font.ttf"})).is_err());
        let junk = dir.join("junk.ttf");
        std::fs::write(&junk, b"definitely not a font").unwrap();
        assert!(s.execute("text.create", &json!({"text": "A", "font": junk})).is_err());
    }

    /// The only test that touches the process-wide font registry, so nothing else can race it.
    #[test]
    fn library_fonts_loaded_by_the_host_work_by_name_and_bad_ones_are_refused() {
        use crate::fontlib as registry;
        let info = r#"{"name":"Loaded Face","horiz_adv_x":{"H":50},"horiz_adv_x_space":30,"leading":98}"#;
        let glyphs = r#"{"document":{"layers":[{"kind":{"type":"layer","children":[{"name":"H","kind":{"type":"path","path":{"subpaths":[{"anchors":[{"p":[0,0]},{"p":[0,-70]},{"p":[50,-70]},{"p":[50,0]}],"closed":true}]}}}]}}]}}"#;
        registry::clear();
        assert!(registry::list().is_empty());
        let mut s = Session::new();
        let err = s.execute("text.create", &json!({"text": "H", "font": "library:loaded"})).unwrap_err().to_string();
        assert!(err.contains("not loaded"), "{err}");

        registry::register("loaded", info, glyphs).unwrap();
        assert_eq!(registry::list(), vec![("library:loaded".to_string(), "Loaded Face".to_string())]);
        let v = s.execute("text.create", &json!({"text": "HH", "x": 5, "y": 5, "height": 10, "font": "library:loaded"})).unwrap();
        let b = s.doc.shape(v["id"].as_u64().unwrap()).unwrap().world_bounds().unwrap();
        assert!((b.y1 - b.y0 - 10.0).abs() < 0.1, "{b:?}");
        // The name is kept with the text, so it can be edited again while the font is loaded.
        assert!(s.execute("text.edit", &json!({"id": v["id"], "text": "H"})).is_ok());

        for bad in ["", ".hidden", "a/b", "a\\b", &"x".repeat(200), "tab\t"] {
            assert!(registry::register(bad, info, glyphs).is_err(), "{bad:?}");
        }
        assert!(registry::register("junk", "{}", "not json").is_err());
        assert!(registry::register("junk", info, "{}").is_err());
        assert_eq!(registry::list().len(), 1, "failed loads leave nothing behind");

        for i in 0..registry::MAX_FONTS {
            let _ = registry::register(&format!("f{i}"), info, glyphs);
        }
        assert_eq!(registry::list().len(), registry::MAX_FONTS);
        assert!(registry::register("one-too-many", info, glyphs).is_err());
        assert!(registry::register("f1", info, glyphs).is_ok(), "replacing a loaded font is always allowed");
        registry::clear();
        assert!(s.execute("text.edit", &json!({"id": v["id"], "text": "H"})).is_err(), "editing needs the font loaded again");
    }

    #[test]
    fn text_can_use_an_inkstitch_font_folder_and_edits_to_its_files_are_picked_up() {
        let dir = std::env::temp_dir().join(format!("lf-engine-inkstitch-{}", std::process::id()));
        let folder = dir.join("tiny");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("font.json"), r#"{"name":"Tiny Satin","horiz_adv_x":{"H":14},"horiz_adv_x_space":9,"leading":30}"#).unwrap();
        let layer =
            |ch: &str, body: &str| format!(r#"<g inkscape:label="GlyphLayer-{ch}" inkscape:groupmode="layer" style="display:none">{body}</g>"#);
        // H: a satin column (two rails, a rung) and a plain bar, drawn 25 units down the page; o: one closed shape.
        let h = layer("H", r#"<path inkstitch:satin_column="True" d="M 2 6 L 2 25 M 8 6 L 8 25 M 2 15 L 8 15"/><path d="M 8 15 L 12 15"/>"#);
        let o = layer("o", r#"<path d="M 1 12 L 9 12 L 9 25 L 1 25 Z"/>"#);
        let svg = |extra: &str| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns:inkstitch="http://inkstitch.org/namespace" viewBox="0 0 30 30">{h}{o}{extra}</svg>"#
            )
        };
        std::fs::write(folder.join("ltr.svg"), svg("")).unwrap();

        let mut s = Session::new();
        let v = s.execute("text.create", &json!({"text": "Ho", "x": 20, "y": 30, "height": 19, "font": folder})).unwrap();
        let id = v["id"].as_u64().unwrap();
        assert!(v["unsupported"].as_array().unwrap().is_empty());
        let b = s.doc.shape(id).unwrap().world_bounds().unwrap();
        assert!((b.y1 - b.y0 - 19.0).abs() < 0.1 && b.x0 >= 19.9, "capitals are the requested height: {b:?}");
        // By file name too.
        assert!(s.execute("text.create", &json!({"text": "H", "font": folder.join("ltr.svg")})).is_ok());
        // Editing a file is noticed (the parsed font is cached by size and time): a third letter appears.
        std::fs::write(folder.join("ltr.svg"), svg(&layer("i", r#"<path d="M 0 12 L 3 12 L 3 25 L 0 25 Z"/>"#))).unwrap();
        let v = s.execute("text.create", &json!({"text": "Hi", "font": folder})).unwrap();
        assert!(v["unsupported"].as_array().unwrap().is_empty(), "{v}");
        // A folder with only a right-to-left drawing is explained, not mistaken for a file.
        let rtl = dir.join("rtl-only");
        std::fs::create_dir_all(&rtl).unwrap();
        std::fs::write(rtl.join("font.json"), "{}").unwrap();
        std::fs::write(rtl.join("rtl.svg"), svg("")).unwrap();
        let err = s.execute("text.create", &json!({"text": "H", "font": rtl})).unwrap_err().to_string();
        assert!(err.contains("not supported yet"), "{err}");
        // A broken drawing is an error.
        std::fs::write(folder.join("ltr.svg"), "<svg>").unwrap();
        assert!(s.execute("text.create", &json!({"text": "H", "font": folder})).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn text_can_use_a_font_library_folder_by_folder_or_by_either_file() {
        let dir = std::env::temp_dir().join(format!("lf-engine-glyphfont-{}", std::process::id()));
        let folder = dir.join("block");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("font.json"), r#"{"name":"Block","horiz_adv_x":{"H":50,"o":40},"horiz_adv_x_space":30,"leading":98}"#).unwrap();
        let glyph = |name: &str, pts: &str| {
            format!(r#"{{"name":"{name}","kind":{{"type":"path","path":{{"subpaths":[{{"anchors":[{pts}],"closed":true}}]}}}}}}"#)
        };
        let h = glyph("H", r#"{"p":[0,0]},{"p":[0,-70]},{"p":[50,-70]},{"p":[50,0]}"#);
        let o = glyph("o", r#"{"p":[0,0]},{"p":[0,-50]},{"p":[40,-50]},{"p":[40,0]}"#);
        std::fs::write(
            folder.join("glyphs.stitchfoundry"),
            format!(r#"{{"document":{{"layers":[{{"kind":{{"type":"layer","children":[{h},{o}]}}}}]}}}}"#),
        )
        .unwrap();

        let mut s = Session::new();
        for font in [folder.clone(), folder.join("font.json"), folder.join("glyphs.stitchfoundry")] {
            let v = s.execute("text.create", &json!({"text": "Ho", "x": 20, "y": 30, "height": 14, "font": font})).unwrap();
            let b = s.doc.shape(v["id"].as_u64().unwrap()).unwrap().world_bounds().unwrap();
            assert!((b.y1 - b.y0 - 14.0).abs() < 0.1 && b.x0 >= 19.9, "{font:?} {b:?}");
            assert!(v["unsupported"].as_array().unwrap().is_empty());
        }
        // A letter the font lacks is reported; a fallback font cannot be a library folder; damaged files are errors.
        let v = s.execute("text.create", &json!({"text": "HZ", "font": folder})).unwrap();
        assert_eq!(v["unsupported"], json!(["Z"]));
        let other = dir.join("tiny.ttf");
        std::fs::write(&other, laserfoundry_text::testfont::tiny_font()).unwrap();
        assert!(s.execute("text.create", &json!({"text": "H", "font": other, "fallback_font": folder})).is_err());
        std::fs::write(folder.join("glyphs.stitchfoundry"), b"{ broken").unwrap();
        assert!(s.execute("text.create", &json!({"text": "H", "font": folder})).is_err());
        std::fs::write(folder.join("glyphs.stitchfoundry"), [0xff, 0xfe, 0x00]).unwrap();
        assert!(s.execute("text.create", &json!({"text": "H", "font": folder})).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_marks_move_and_turn_the_whole_design_onto_where_they_really_are() {
        let mut s = Session::new();
        let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 4, "height": 4})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": 106, "y": 10, "width": 4, "height": 4})).unwrap()["id"].as_u64().unwrap();
        s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 10, "height": 10})).unwrap();
        // The sheet was put down 5 mm to the right and 2 mm lower, and not turned.
        let v = s.execute("align.two_point", &json!({"design": [[12, 12], [108, 12]], "actual": [[17, 14], [113, 14]]})).unwrap();
        assert_eq!(v["moved"], json!(3));
        assert!(v["rotation_deg"].as_f64().unwrap().abs() < 1e-9);
        let ba = s.doc.shape(a).unwrap().world_bounds().unwrap();
        assert!((ba.x0 - 15.0).abs() < 1e-9 && (ba.y0 - 12.0).abs() < 1e-9, "{ba:?}");
        let bb = s.doc.shape(b).unwrap().world_bounds().unwrap();
        assert!((bb.x0 - 111.0).abs() < 1e-9);

        // Only some shapes, with a turn.
        s.execute("align.two_point", &json!({"design": [[0, 0], [100, 0]], "actual": [[0, 0], [0, 100]], "ids": [a]})).unwrap();
        let rotated = s.doc.shape(a).unwrap().world_bounds().unwrap();
        assert!((rotated.x0 - -16.0).abs() < 1e-6 && (rotated.y0 - 15.0).abs() < 1e-6, "{rotated:?}");
        assert!((s.doc.shape(b).unwrap().world_bounds().unwrap().x0 - 111.0).abs() < 1e-9, "others stay");
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!((s.doc.shape(a).unwrap().world_bounds().unwrap().x0 - 15.0).abs() < 1e-9, "one undo step");

        for bad in [
            json!({"design": [[1, 1], [1, 1]], "actual": [[0, 0], [5, 0]]}),
            json!({"design": [[0, 0], [1, 0]], "actual": [[0, 0], [900, 0]], "lock_scale": false}),
            json!({"design": [[0, 0]], "actual": [[0, 0], [5, 0]]}),
            json!({"design": [[0, 0], [5, 0]], "actual": [[0, 0], [5, 0]], "ids": [999]}),
        ] {
            assert!(s.execute("align.two_point", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn projects_and_pictures_can_travel_as_inline_data() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"width": 12, "height": 7})).unwrap();
        let v = s.execute("file.project_data", &Value::Null).unwrap();
        assert!(!s.dirty);
        let data = v["data_base64"].as_str().unwrap().to_string();
        let mut other = Session::new();
        other.execute("file.open", &json!({"data_base64": data})).unwrap();
        assert_eq!(other.doc.shapes.len(), 1);
        assert!(other.path.is_none(), "opened from data, so it has no path to save back to");
        assert!(other.execute("file.open", &json!({"data_base64": "!!!"})).is_err());
        assert!(other.execute("file.open", &json!({"data_base64": "e30="})).is_err(), "valid base64 of something that is not a project");
        assert!(other.execute("file.open", &json!({})).is_err());

        let mut png = Vec::new();
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(8, 4, image::Luma([0])))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let v = other
            .execute("file.import_image", &json!({"data_base64": laserfoundry_doc::base64::encode(&png), "name": "dot.png", "width": 16}))
            .unwrap();
        assert_eq!(v["height_mm"], json!(8.0));
        assert_eq!(other.doc.shape(v["id"].as_u64().unwrap()).unwrap().name, "dot.png");
        assert!(other.execute("file.import_image", &json!({"data_base64": "AAAA"})).is_err(), "not a picture");
        assert!(other.execute("file.import_image", &json!({})).is_err());
    }

    #[test]
    fn overscan_and_ramp_are_layer_settings_with_validation() {
        let mut s = Session::new();
        s.execute("layer.set", &json!({"id": 0, "overscan": 4, "ramp_length": 6})).unwrap();
        assert_eq!((s.doc.layer(0).unwrap().overscan, s.doc.layer(0).unwrap().ramp_length), (Some(4.0), 6.0));
        s.execute("layer.set", &json!({"id": 0, "default_overscan": true})).unwrap();
        assert_eq!(s.doc.layer(0).unwrap().overscan, None);
        for bad in [json!({"overscan": 99}), json!({"overscan": -1}), json!({"ramp_length": 1e6}), json!({"ramp_length": "x"})] {
            let mut p = bad.clone();
            p["id"] = json!(0);
            assert!(s.execute("layer.set", &p).is_err(), "{bad}");
        }
    }

    #[test]
    fn resampling_changes_pixels_not_size_and_is_validated() {
        let dir = std::env::temp_dir().join(format!("lf-engine-resample-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.png");
        image::GrayImage::from_fn(40, 20, |x, _| image::Luma([(x * 6) as u8])).save(&path).unwrap();
        let mut s = Session::new();
        let id = s.execute("file.import_image", &json!({"path": path, "width": 80})).unwrap()["id"].as_u64().unwrap();
        let before = s.doc.shape(id).unwrap().world_bounds().unwrap();
        let v = s.execute("image.resample", &json!({"scale": 0.5})).unwrap();
        assert_eq!((v["width_px"].as_u64(), v["height_px"].as_u64()), (Some(20), Some(10)));
        assert_eq!(s.doc.shape(id).unwrap().world_bounds().unwrap(), before, "the size on the bed is unchanged");
        s.execute("image.resample", &json!({"scale": 3})).unwrap();
        for bad in [json!({"scale": 0.0}), json!({"scale": 5}), json!({"scale": -1}), json!({"scale": "x"}), json!({})] {
            assert!(s.execute("image.resample", &bad).is_err(), "{bad}");
        }
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        let laserfoundry_doc::ShapeKind::Image { image, .. } = &s.doc.shape(id).unwrap().kind else { panic!("not an image") };
        assert_eq!(image.width, 40);
    }

    #[test]
    fn text_can_be_bent_along_a_circle_or_a_line() {
        let mut s = Session::new();
        let text = s.execute("text.create", &json!({"text": "HELLO", "x": 0, "y": 0, "height": 10})).unwrap()["id"].as_u64().unwrap();
        let tb = s.doc.shape(text).unwrap().world_bounds().unwrap();
        let circle = s.execute("shape.ellipse", &json!({"cx": 100, "cy": 100, "rx": 40, "ry": 40})).unwrap()["id"].as_u64().unwrap();
        let v = s.execute("text.on_path", &json!({"text": text, "path": circle, "offset": 0})).unwrap();
        let bent = s.doc.shape(v["id"].as_u64().unwrap()).unwrap();
        assert_eq!(s.doc.shapes.len(), 3, "the originals are kept by default");
        // Every point of the bent text lies between the circle and 10 mm outside it (the letters stand on the line).
        for pl in bent.world_polylines() {
            for p in &pl.points {
                let r = p.distance(laserfoundry_geom::Point::new(100.0, 100.0));
                assert!(r > 39.0 && r < 50.5, "radius {r}");
            }
        }
        let bb = bent.world_bounds().unwrap();
        assert!(bb.height() > tb.height() * 2.0, "the word is no longer a flat strip: {bb:?}");

        // On a straight line the text stays straight, just placed on it.
        let line = s.execute("shape.line", &json!({"x1": 10, "y1": 200, "x2": 200, "y2": 200})).unwrap()["id"].as_u64().unwrap();
        let v = s.execute("text.on_path", &json!({"text": text, "path": line, "offset": 5, "keep": false})).unwrap();
        assert!(s.doc.shape(text).is_none(), "keep: false removes the text");
        let flat = s.doc.shape(v["id"].as_u64().unwrap()).unwrap().world_bounds().unwrap();
        assert!((flat.y1 - 200.0).abs() < 0.2 && (flat.y1 - flat.y0 - tb.height()).abs() < 0.3, "{flat:?}");
        assert!((flat.x0 - 15.0).abs() < 0.5, "the offset moves the start along the line: {flat:?}");
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shape(text).is_some(), "one undo step");

        assert!(s.execute("text.on_path", &json!({"text": text, "path": text})).is_err());
        assert!(s.execute("text.on_path", &json!({"text": circle, "path": line})).is_err(), "a circle is not text");
        assert!(s.execute("text.on_path", &json!({"text": text, "path": 9999})).is_err());
        let _ = s.execute("text.on_path", &json!({"text": text, "path": line, "offset": 1e30})); // absurd offsets may not panic
    }

    #[test]
    fn group_and_start_options_are_validated_and_one_undo_step() {
        let mut s = Session::new();
        let v = s.execute("job.set_optimize", &json!({"by_group": true, "start": [120, 30]})).unwrap();
        assert_eq!((v["by_group"].clone(), v["start"].clone()), (json!(true), json!([120.0, 30.0])));
        for bad in [json!({"start": [1e12, 0]}), json!({"start": [1]}), json!({"start": "here"}), json!({"by_group": "yes"})] {
            assert!(s.execute("job.set_optimize", &bad).is_err(), "{bad}");
        }
        s.execute("job.set_optimize", &json!({"default_start": true})).unwrap();
        assert_eq!(s.doc.optimize.start, None);
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.optimize.start, Some([120.0, 30.0]));
    }

    #[test]
    fn smooth_tracing_of_a_disc_gives_curves_with_fewer_nodes_than_corners() {
        let dir = std::env::temp_dir().join(format!("lf-engine-smooth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("disc.png");
        image::GrayImage::from_fn(60, 60, |x, y| {
            let (dx, dy) = (f64::from(x) - 29.5, f64::from(y) - 29.5);
            image::Luma([if dx * dx + dy * dy < 20.0 * 20.0 { 0 } else { 255 }])
        })
        .save(&path)
        .unwrap();
        let mut s = Session::new();
        let pic = s.execute("file.import_image", &json!({"path": path, "width": 60})).unwrap()["id"].as_u64().unwrap();
        let count = |s: &Session, id: u64| match &s.doc.shape(id).unwrap().kind {
            laserfoundry_doc::ShapeKind::Path { path } => path.elements().iter().filter(|e| matches!(e, kurbo::PathEl::CurveTo(..))).count(),
            other => panic!("{other:?}"),
        };
        let plain = s.execute("image.trace", &json!({"id": pic, "tolerance": 1.0})).unwrap()["id"].as_u64().unwrap();
        let smooth = s.execute("image.trace", &json!({"id": pic, "tolerance": 1.0, "smooth": true})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(count(&s, plain), 0, "straight steps");
        assert!(count(&s, smooth) > 8, "curves: {}", count(&s, smooth));
        let (a, b) = (s.doc.shape(plain).unwrap().world_bounds().unwrap(), s.doc.shape(smooth).unwrap().world_bounds().unwrap());
        assert!((a.width() - b.width()).abs() < 1.5 && (a.height() - b.height()).abs() < 1.5, "{a:?} vs {b:?}");
        s.execute("node.insert", &json!({"id": smooth, "segment": 0, "t": 0.5})).unwrap(); // and the result is editable
    }

    #[test]
    fn svg_pictures_are_imported_in_place_and_text_does_not_break_the_import() {
        let mut png = Vec::new();
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(10, 5, image::Luma([0])))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let b64 = laserfoundry_doc::base64::encode(&png);
        // 96 px per inch: 96 px = 25.4 mm. The picture is 48 x 24 px at (96, 48), a rectangle sits beside it.
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="300" height="150" viewBox="0 0 300 150"><rect x="0" y="0" width="20" height="20"/><image x="96" y="48" width="48" height="24" xlink:href="data:image/png;base64,{b64}"/><text x="10" y="100" font-size="20">Hi</text></svg>"#
        );
        let mut s = Session::new();
        let v = s.execute("file.import_svg", &json!({"data": svg, "x": 10, "y": 20})).unwrap();
        assert_eq!(v["skipped_images"], json!(0));
        let pic = s.doc.shapes.iter().find(|sh| matches!(sh.kind, laserfoundry_doc::ShapeKind::Image { .. })).expect("the picture was imported");
        let b = pic.world_bounds().unwrap();
        let mm = 25.4 / 96.0;
        assert!((b.x0 - (10.0 + 96.0 * mm)).abs() < 1e-6 && (b.y0 - (20.0 + 48.0 * mm)).abs() < 1e-6, "{b:?}");
        assert!((b.width() - 48.0 * mm).abs() < 1e-6 && (b.height() - 24.0 * mm).abs() < 1e-6, "{b:?}");

        let broken = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="50" height="50"><image width="10" height="10" xlink:href="data:image/png;base64,AAAA"/><rect width="5" height="5"/></svg>"#;
        let mut t = Session::new();
        assert!(t.execute("file.import_svg", &json!({"data": broken})).is_ok(), "an undecodable picture is not fatal");
        assert!(!t.doc.shapes.is_empty());
    }

    #[test]
    fn acceleration_is_a_validated_machine_setting_that_lengthens_the_estimate() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let secs = |s: &mut Session| s.execute("job.estimate", &Value::Null).unwrap()["seconds"].as_f64().unwrap();
        let plain = secs(&mut s);
        s.execute("machine.set", &json!({"acceleration": 100})).unwrap();
        assert!(secs(&mut s) > plain);
        for bad in [json!({"acceleration": -1}), json!({"acceleration": 1e9}), json!({"acceleration": "fast"})] {
            assert!(s.execute("machine.set", &bad).is_err(), "{bad}");
        }
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(secs(&mut s), plain);
    }

    #[test]
    fn junction_deviation_is_validated_and_sets_how_fast_corners_are_taken_in_the_estimate() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        s.execute("machine.set", &json!({"acceleration": 100})).unwrap();
        let secs = |s: &mut Session| s.execute("job.estimate", &Value::Null).unwrap()["seconds"].as_f64().unwrap();
        let default = secs(&mut s);
        s.execute("machine.set", &json!({"junction_deviation": 0})).unwrap();
        let stopping = secs(&mut s);
        assert!(stopping > default, "stopping at every corner is slower: {stopping} vs {default}");
        s.execute("machine.set", &json!({"junction_deviation": 5})).unwrap();
        assert!(secs(&mut s) < default, "a generous deviation lets the head through the corners faster");
        for bad in [json!({"junction_deviation": -0.1}), json!({"junction_deviation": 11}), json!({"junction_deviation": "tight"})] {
            assert!(s.execute("machine.set", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.doc.machine.junction_deviation, 5.0, "a refused value changes nothing");
        // Without an acceleration the setting does nothing.
        s.execute("machine.set", &json!({"acceleration": 0})).unwrap();
        let none = secs(&mut s);
        s.execute("machine.set", &json!({"junction_deviation": 0.01})).unwrap();
        assert_eq!(secs(&mut s), none);
    }

    #[test]
    fn cut_order_is_validated_applies_to_the_selection_and_undoes() {
        let mut s = Session::new();
        let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("shape.set_order", &json!({"order": 3})).unwrap();
        assert_eq!(s.doc.shape(a).unwrap().order, 3);
        for bad in [json!({"order": 5000}), json!({"order": -1001}), json!({"order": "x"}), json!({})] {
            assert!(s.execute("shape.set_order", &bad).is_err(), "{bad}");
        }
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.shape(a).unwrap().order, 0);
        s.selection.clear();
        assert!(s.execute("shape.set_order", &json!({"order": 1})).is_err(), "nothing selected");
    }

    #[test]
    fn vertical_text_is_a_toggle_that_changes_the_shape_and_undoes() {
        let mut s = Session::new();
        let id = s.execute("text.create", &json!({"text": "ABC", "height": 10})).unwrap()["id"].as_u64().unwrap();
        let size = |s: &Session| {
            let b = s.doc.shape(id).unwrap().world_bounds().unwrap();
            (b.width(), b.height())
        };
        let (w, h) = size(&s);
        assert!(w > h);
        s.execute("text.edit", &json!({"id": id, "vertical": true})).unwrap();
        let (w2, h2) = size(&s);
        assert!(h2 > w2, "stacked: {w2} x {h2}");
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(size(&s), (w, h));
    }

    #[test]
    fn overscan_by_speed_is_a_machine_switch_that_undoes() {
        let mut s = Session::new();
        s.execute("machine.set", &json!({"overscan_by_speed": true, "acceleration": 500})).unwrap();
        assert!(s.doc.machine.overscan_by_speed);
        assert!(s.execute("machine.set", &json!({"overscan_by_speed": "yes"})).is_err());
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(!s.doc.machine.overscan_by_speed);
    }

    #[test]
    fn opening_a_file_as_a_template_leaves_an_untitled_unsaved_project() {
        let dir = std::env::temp_dir().join(format!("lf-engine-template-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.laserfoundry");
        let mut a = Session::new();
        a.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap();
        a.execute("file.save", &json!({"path": file})).unwrap();
        let mut b = Session::new();
        b.execute("file.open", &json!({"path": file, "template": true})).unwrap();
        assert_eq!(b.doc.shapes.len(), 1);
        assert!(b.path.is_none() && b.dirty);
        assert!(b.execute("file.save", &json!({})).is_err(), "no path to save over the template");
        b.execute("file.open", &json!({"path": file})).unwrap();
        assert_eq!(b.path.as_deref(), Some(file.as_path()));
    }

    #[test]
    fn every_dither_mode_can_be_chosen_by_name_and_unknown_ones_are_refused() {
        let mut s = Session::new();
        for name in ["threshold", "floyd_steinberg", "grayscale", "atkinson", "jarvis", "ordered", "halftone"] {
            s.execute("layer.set", &json!({"id": 0, "dither": name})).unwrap();
        }
        assert_eq!(s.doc.layer(0).unwrap().dither, laserfoundry_doc::Dither::Halftone);
        assert!(s.execute("layer.set", &json!({"id": 0, "dither": "stucki"})).is_err());
    }

    #[test]
    fn cropping_keeps_the_remaining_picture_in_place_and_sharpening_is_validated() {
        let dir = std::env::temp_dir().join(format!("lf-engine-crop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.png");
        image::GrayImage::from_fn(20, 10, |x, y| image::Luma([(x * 10 + y) as u8])).save(&path).unwrap();
        let mut s = Session::new();
        let id = s.execute("file.import_image", &json!({"path": path, "width": 40, "x": 100, "y": 50})).unwrap()["id"].as_u64().unwrap();
        let bounds = |s: &Session| s.doc.shape(id).unwrap().world_bounds().unwrap();
        let before = bounds(&s);
        let pixel_at = |s: &Session, col: u32, row: u32| match &s.doc.shape(id).unwrap().kind {
            laserfoundry_doc::ShapeKind::Image { image, .. } => image.gray[(row * image.width + col) as usize],
            other => panic!("{other:?}"),
        };
        let marker = pixel_at(&s, 10, 5);

        let v = s.execute("image.crop", &json!({"left": 0.5, "top": 0.2, "bottom": 0.2})).unwrap();
        assert!((v["width_mm"].as_f64().unwrap() - 20.0).abs() < 1e-9, "{v}");
        let after = bounds(&s);
        assert!((after.x0 - (before.x0 + 20.0)).abs() < 1e-6 && (after.x1 - before.x1).abs() < 1e-6, "{before:?} -> {after:?}");
        assert!(after.y0 > before.y0 && after.y1 < before.y1);
        assert_eq!(pixel_at(&s, 0, 3), marker, "the pixel that was at column 10, row 5 is now at column 0, row 3");

        for bad in [
            json!({"left": 0.6, "right": 0.6}),
            json!({"top": 1.0}),
            json!({"left": -0.1}),
            json!({"left": f64::NAN}),
            json!({"left": "a"}),
            json!({"right": 0.99}),
        ] {
            assert!(s.execute("image.crop", &bad).is_err(), "{bad}");
        }
        s.execute("image.sharpen", &json!({"amount": 2.0})).unwrap();
        assert!(s.execute("image.sharpen", &json!({"amount": 9})).is_err());
        assert!(s.execute("image.sharpen", &json!({})).is_err());
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(bounds(&s), before, "undo restores the whole picture");
        s.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap();
        assert!(s.execute("image.crop", &json!({"left": 0.1})).is_err(), "a rectangle is not an image");
    }

    #[test]
    fn image_adjustments_are_validated_undoable_and_saved() {
        let dir = std::env::temp_dir().join(format!("lf-engine-adjust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("g.png");
        image::GrayImage::from_fn(8, 8, |x, _| image::Luma([(x * 30) as u8])).save(&path).unwrap();
        let mut s = Session::new();
        let id = s.execute("file.import_image", &json!({"path": path, "width": 8})).unwrap()["id"].as_u64().unwrap();
        s.execute("image.adjust", &json!({"brightness": 0.25, "gamma": 1.8, "negative": true})).unwrap();
        for bad in [json!({"brightness": 2}), json!({"contrast": -3}), json!({"gamma": 0}), json!({"gamma": "x"})] {
            assert!(s.execute("image.adjust", &bad).is_err(), "{bad}");
        }
        let back = laserfoundry_format::from_bytes(&laserfoundry_format::to_bytes(&s.doc).unwrap()).unwrap();
        match &back.shape(id).unwrap().kind {
            laserfoundry_doc::ShapeKind::Image { adjust, .. } => assert!(adjust.negative && adjust.brightness == 0.25 && adjust.gamma == 1.8),
            other => panic!("{other:?}"),
        }
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("image.adjust", &json!({"reset": true})).unwrap();
        s.execute("shape.rectangle", &json!({"width": 5, "height": 5})).unwrap();
        assert!(s.execute("image.adjust", &json!({})).is_err(), "a rectangle is not an image");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn png_import_becomes_a_raster_shape() {
        let dir = std::env::temp_dir().join(format!("lf-engine-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.png");
        image::GrayImage::from_fn(10, 5, |x, _| image::Luma([(x * 25) as u8])).save(&path).unwrap();
        let mut s = Session::new();
        let v = s.execute("file.import_image", &json!({"path": path, "width": 20})).unwrap();
        assert!((v["height_mm"].as_f64().unwrap() - 10.0).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod job_tests {
    use super::*;

    fn square_session() -> Session {
        let mut s = Session::new();
        s.execute("machine.set", &json!({"origin": "top_left"})).unwrap();
        rect(&mut s);
        s
    }

    #[test]
    fn gcode_is_generated_and_streams_to_the_simulator() {
        let mut s = square_session();
        let v = s.execute("job.gcode", &json!({"include": true})).unwrap();
        assert!(v["gcode"].as_str().unwrap().contains("G1"));
        assert!(s.last_gcode.is_some());
        let sim = s.execute("job.simulate", &Value::Null).unwrap();
        assert_eq!(sim["overran_buffer"], json!(false));
        assert!(sim["lines_streamed"].as_u64().unwrap() > 5);
    }

    #[test]
    fn cut_order_options_are_one_undo_step_and_change_the_program() {
        let mut s = square_session();
        rect(&mut s); // the same rectangle again, exactly on top
        let count = |s: &mut Session| s.execute("job.gcode", &json!({"include": true})).unwrap()["gcode"].as_str().unwrap().lines().count();
        let deduplicated = count(&mut s);
        let v = s.execute("job.set_optimize", &json!({"remove_duplicates": false})).unwrap();
        assert_eq!(v["remove_duplicates"], json!(false));
        assert_eq!(v["inner_first"], json!(true), "unmentioned options are left alone");
        assert!(count(&mut s) > deduplicated, "the copy on top is now cut as well");
        assert!(s.undo());
        assert!(s.doc.optimize.remove_duplicates);
        assert!(s.execute("job.set_optimize", &json!({"inner_first": "yes"})).is_err());
    }

    #[test]
    fn estimate_and_frame_work() {
        let mut s = square_session();
        let e = s.execute("job.estimate", &Value::Null).unwrap();
        assert!(e["seconds"].as_f64().unwrap() > 0.0);
        let layers = e["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 1, "one layer is used");
        assert!((layers[0]["seconds"].as_f64().unwrap() - e["seconds"].as_f64().unwrap()).abs() < 1e-9);
        assert!(!layers[0]["name"].as_str().unwrap().is_empty());
        let f = s.execute("job.frame", &json!({})).unwrap();
        assert_eq!(f["bounds"], json!([10.0, 10.0, 30.0, 20.0]));
    }

    #[test]
    fn out_of_bed_designs_are_refused() {
        let mut s = square_session();
        s.execute("edit.move", &json!({"dx": 1000, "dy": 0})).unwrap();
        assert!(matches!(s.execute("job.gcode", &Value::Null), Err(EngineError::Gcode(_))));
    }

    #[test]
    fn start_from_current_position_makes_the_job_relative_to_its_anchor() {
        let mut s = square_session(); // 20x10 rect at (10,10), top-left origin
        s.execute("job.set_origin", &json!({"start_from": "current_position", "anchor": "center"})).unwrap();
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
        let text = g["gcode"].as_str().unwrap();
        assert!(text.contains("G92 X0 Y0"), "{text}");
        assert!(text.contains("X-10") && text.contains("Y-5"), "centre anchor: corner is at (-10,-5)\n{text}");
        // Even a job far outside the design bed is fine when placement is relative.
        s.execute("edit.move", &json!({"dx": 5000, "dy": 0})).unwrap();
        assert!(s.execute("job.gcode", &Value::Null).is_ok());
        s.execute("job.set_origin", &json!({"start_from": "absolute"})).unwrap();
        assert!(matches!(s.execute("job.gcode", &Value::Null), Err(EngineError::Gcode(_))));
    }

    #[test]
    fn job_origin_is_undoable_and_saved() {
        let mut s = square_session();
        s.execute("job.set_origin", &json!({"anchor": "bottom_right"})).unwrap();
        let bytes = laserfoundry_format::to_bytes(&s.doc).unwrap();
        assert_eq!(laserfoundry_format::from_bytes(&bytes).unwrap().job_origin.anchor, laserfoundry_doc::Anchor::BottomRight);
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.job_origin.anchor, laserfoundry_doc::Anchor::TopLeft);
    }

    #[test]
    fn layer_settings_flow_through_to_gcode() {
        let mut s = square_session();
        s.execute("layer.set", &json!({"id": 0, "speed": 777, "max_power": 25, "passes": 2})).unwrap();
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
        let text = g["gcode"].as_str().unwrap();
        assert!(text.contains("F777") && text.contains("S250"));
        assert_eq!(text.matches("pass").count(), 2);
    }

    #[test]
    fn tabs_and_leads_reach_the_gcode_and_are_validated() {
        let mut s = square_session(); // 20x10 rectangle
        s.execute("layer.set", &json!({"id": 0, "tab_count": 2, "tab_width": 1.5, "lead_in": 3})).unwrap();
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
        let text = g["gcode"].as_str().unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("G1") && l.contains(" S0")).count(), 2, "one laser-off move per tab\n{text}");
        assert!(text.contains("G0 X") && text.lines().filter(|l| l.starts_with("G0")).count() >= 2, "the lead-in start is a rapid move");
        for bad in [
            json!({"id": 0, "tab_width": -1}),
            json!({"id": 0, "lead_in": 1e9}),
            json!({"id": 0, "lead_out": "x"}),
            json!({"id": 0, "tab_count": -3}),
        ] {
            assert!(s.execute("layer.set", &bad).is_err(), "{bad}");
        }
        s.execute("layer.set", &json!({"id": 0, "tab_count": 5000})).unwrap();
        assert_eq!(s.doc.layer(0).unwrap().tab_count, 1000, "counts are capped");
    }

    #[test]
    fn kerf_offset_moves_the_generated_cut_path_and_is_validated() {
        let mut s = square_session(); // 20x10 rect at (10,10), top-left origin
        s.execute("layer.set", &json!({"id": 0, "kerf_offset": 0.5})).unwrap();
        let g = s.execute("job.gcode", &json!({"include": true})).unwrap();
        let text = g["gcode"].as_str().unwrap();
        assert!(text.contains("X9.5") && text.contains("X30.5"), "the outline is cut 0.5 mm outside the drawn rectangle\n{text}");
        assert!(s.execute("layer.set", &json!({"id": 0, "kerf_offset": 99})).is_err());
        assert!(s.execute("layer.set", &json!({"id": 0, "kerf_offset": "wide"})).is_err());
    }

    #[test]
    fn layer_and_machine_settings_are_validated() {
        let mut s = Session::new();
        for bad in [
            json!({"id": 0, "speed": 0}),
            json!({"id": 0, "max_power": 101}),
            json!({"id": 0, "interval": -1}),
            json!({"id": 99, "speed": 10}),
            json!({"id": 0, "speed": null, "passes": "x"}),
        ] {
            assert!(s.execute("layer.set", &bad).is_err(), "{bad}");
        }
        assert!(s.execute("machine.set", &json!({"bed_width": 0})).is_err());
        assert!(s.execute("machine.set", &json!({"max_s": 0})).is_err());
    }
}

mod file_tests {
    use super::*;

    #[test]
    fn save_and_open_round_trip() {
        let dir = std::env::temp_dir().join(format!("lf-engine-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.laserfoundry");
        let mut s = Session::new();
        rect(&mut s);
        s.execute("file.save", &json!({"path": path})).unwrap();
        assert!(!s.dirty);
        let mut t = Session::new();
        t.execute("file.open", &json!({"path": path})).unwrap();
        assert_eq!(t.doc, s.doc);
        assert!(!t.can_undo(), "history does not carry across documents");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_leaves_the_document_state_alone_and_recover_reopens_it_as_unsaved() {
        let dir = std::env::temp_dir().join(format!("lf-engine-recover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (saved, recovery) = (dir.join("real.laserfoundry"), dir.join("recovery.laserfoundry"));
        let mut s = Session::new();
        rect(&mut s);
        s.execute("file.save", &json!({"path": saved})).unwrap();
        rect(&mut s);
        assert!(s.dirty);
        s.execute("file.autosave", &json!({"path": recovery})).unwrap();
        assert!(s.dirty, "autosave does not mark the document clean");
        assert_eq!(s.path.as_deref(), Some(saved.as_path()), "nor does it retarget Save");

        let mut t = Session::new();
        t.execute("file.recover", &json!({"path": recovery})).unwrap();
        assert_eq!(t.doc, s.doc);
        assert!(t.dirty && t.path.is_none(), "recovered work is untitled and unsaved");
        assert!(Session::new().execute("file.recover", &json!({"path": dir.join("missing")})).is_err());
        std::fs::write(dir.join("junk"), b"not a project").unwrap();
        assert!(Session::new().execute("file.recover", &json!({"path": dir.join("junk")})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_without_a_path_is_an_error() {
        assert!(Session::new().execute("file.save", &json!({})).is_err());
    }
}

mod robustness_tests {
    use super::*;

    #[test]
    fn unknown_commands_are_errors() {
        assert!(matches!(Session::new().execute("nope.nothing", &Value::Null), Err(EngineError::UnknownCommand(_))));
    }

    #[test]
    fn a_panicking_command_is_contained_and_rolls_back() {
        fn boom(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
            s.doc.shapes.clear();
            s.selection.clear();
            panic!("kaboom");
        }
        let spec = crate::CommandSpec { id: "test.boom", label: "Boom", menu: "", shortcut: None, params_doc: "{}", mutates: true, run: boom };
        let mut s = Session::new();
        rect(&mut s);
        let before = s.doc.clone();
        let selection = s.selection.clone();
        let err = s.execute_spec_for_test(&spec, &Value::Null).unwrap_err();
        assert!(matches!(&err, EngineError::Internal(m) if m.contains("kaboom")), "{err}");
        assert_eq!(s.doc, before, "the document must be restored after a panic");
        assert_eq!(s.selection, selection);
        // The failed command is not an undo step: one undo removes only the rectangle.
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty());
    }

    /// Arrays copy every selected shape: the total, not just the grid size, is bounded.
    #[test]
    fn an_array_of_many_shapes_cannot_multiply_into_millions() {
        let mut s = Session::new();
        for i in 0..30 {
            s.execute("shape.rectangle", &json!({"x": i, "y": 0, "width": 1, "height": 1})).unwrap();
        }
        s.execute("edit.select_all", &Value::Null).unwrap();
        let before = s.doc.shapes.len();
        assert!(s.execute("arrange.grid", &json!({"columns": 100, "rows": 100})).is_err());
        assert!(s.execute("arrange.circular", &json!({"count": 9000})).is_err());
        assert_eq!(s.doc.shapes.len(), before, "a refused array changes nothing");
        assert!(s.execute("arrange.grid", &json!({"columns": 2, "rows": 2})).is_ok());
    }

    const ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="20mm" viewBox="0 0 40 20">
        <rect x="0" y="0" width="40" height="20" fill="none" stroke="#ff0000"/>
        <circle cx="10" cy="10" r="4" fill="#000000"/><path d="M20 5 L30 15" stroke="#0000ff" fill="none"/></svg>"##;

    #[test]
    fn ai_artwork_lands_on_the_layers_its_colours_name_centred_and_grouped() {
        let mut s = Session::new();
        let v = s.execute("ai.place_art", &json!({ "svg": ART })).unwrap();
        assert_eq!(v["ids"].as_array().unwrap().len(), 3);
        assert_eq!(v["layers"], json!([0, 1, 2]), "black, blue and red are the first layers");
        assert!((v["width_mm"].as_f64().unwrap() - 40.0).abs() < 0.5, "{v}");
        let bed = (s.doc.machine.bed_width, s.doc.machine.bed_height);
        let b = s.doc.bounds_of(&s.selection).unwrap();
        assert!((b.center().x - bed.0 / 2.0).abs() < 0.5 && (b.center().y - bed.1 / 2.0).abs() < 0.5, "centred on the bed");
        assert_eq!(s.doc.layer(0).unwrap().mode, laserfoundry_doc::LayerMode::Fill, "an untouched layer of filled art engraves");
        assert_eq!(s.doc.layer(2).unwrap().mode, laserfoundry_doc::LayerMode::Line);
        let groups: std::collections::BTreeSet<_> = s.doc.shapes.iter().map(|sh| sh.group).collect();
        assert_eq!(groups.len(), 1, "one group");
        // one undo step takes all of it away
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty());
    }

    #[test]
    fn ai_artwork_can_be_sized_placed_and_is_kept_on_the_bed() {
        let mut s = Session::new();
        s.execute("ai.place_art", &json!({ "svg": ART, "size_mm": 80, "x": 10, "y": 20 })).unwrap();
        let b = s.doc.bounds_of(&s.selection).unwrap();
        assert!((b.width() - 80.0).abs() < 0.5 && (b.x0 - 10.0).abs() < 0.01 && (b.y0 - 20.0).abs() < 0.01, "{b:?}");
        // as drawn, but never larger than the bed
        let big = ART.replace("40mm", "4000mm").replace("20mm", "2000mm").replace("0 0 40 20", "0 0 4000 2000");
        let mut s = Session::new();
        s.execute("ai.place_art", &json!({ "svg": big })).unwrap();
        let b = s.doc.bounds_of(&s.selection).unwrap();
        assert!(b.width() <= s.doc.machine.bed_width + 0.01 && b.height() <= s.doc.machine.bed_height + 0.01, "{b:?}");
    }

    #[test]
    fn ai_artwork_that_cannot_be_used_is_refused_and_changes_nothing() {
        let mut s = Session::new();
        for bad in [
            json!({ "svg": "not an svg" }),
            json!({ "svg": "<svg xmlns='http://www.w3.org/2000/svg'/>" }),
            json!({ "svg": ART, "size_mm": 0 }),
            json!({ "svg": ART, "size_mm": 1e12 }),
            json!({ "svg": ART, "x": 1e308, "y": null }),
            json!({}),
        ] {
            assert!(s.execute("ai.place_art", &bad).is_err(), "{bad}");
        }
        assert!(s.doc.shapes.is_empty());
    }

    #[test]
    fn pictures_for_the_ai_are_shrunk_flattened_onto_white_and_encoded() {
        let mut s = Session::new();
        let mut png = Vec::new();
        // transparent except for a black square: 3000 x 1000
        let mut img = image::RgbaImage::from_pixel(3000, 1000, image::Rgba([0, 0, 0, 0]));
        for x in 100..200 {
            for y in 100..200 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let v = s.execute("ai.prepare_picture", &json!({ "data_base64": laserfoundry_doc::base64::encode(&png) })).unwrap();
        assert_eq!(v["media_type"], "image/jpeg");
        assert_eq!(v["width"], 1568);
        assert!((v["height"].as_u64().unwrap() as i64 - 523).abs() <= 1, "{v}");
        let jpeg = laserfoundry_doc::base64::decode(v["data_base64"].as_str().unwrap()).unwrap();
        let out = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        assert!(out.get_pixel(1000, 400).0.iter().all(|c| *c > 240), "transparent became white");
        assert!(out.get_pixel(75, 75).0.iter().all(|c| *c < 40), "the black square stayed black");
        // not a picture, nothing given, and an endless file
        assert!(s.execute("ai.prepare_picture", &json!({ "data_base64": "AAAA" })).is_err());
        assert!(s.execute("ai.prepare_picture", &json!({})).is_err());
        #[cfg(unix)]
        assert!(s.execute("ai.prepare_picture", &json!({ "path": "/dev/zero" })).is_err());
    }

    #[test]
    fn the_model_is_told_about_the_machine_and_the_layers_in_use() {
        let mut s = Session::new();
        assert!(s.execute("ai.context", &Value::Null).unwrap()["text"].as_str().unwrap().contains("The document is empty"));
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("layer.set", &json!({"id": 0, "speed": 750, "max_power": 55, "passes": 2, "air_assist": true})).unwrap();
        let t = s.execute("ai.context", &Value::Null).unwrap()["text"].as_str().unwrap().to_string();
        assert!(t.contains("bed 400 x 400") || t.contains("bed "), "{t}");
        assert!(
            t.contains("Layer 0") && t.contains("speed 750 mm/min") && t.contains("55 %") && t.contains("2 passes") && t.contains("air assist on"),
            "{t}"
        );
        assert!(!t.contains("Layer 1"), "only layers in use");
    }

    /// A path that names an endless device must be refused, not read until memory runs out.
    #[cfg(unix)]
    #[test]
    fn importing_from_an_endless_file_is_refused() {
        let mut s = Session::new();
        for (id, params) in [
            ("file.import_svg", json!({ "path": "/dev/zero" })),
            ("file.import_dxf", json!({ "path": "/dev/zero" })),
            ("file.import_image", json!({ "path": "/dev/zero" })),
            ("file.open", json!({ "path": "/dev/zero" })),
        ] {
            assert!(s.execute(id, &params).is_err(), "{id}");
        }
        assert!(s.doc.shapes.is_empty());
    }

    /// Every command, with a menagerie of hostile parameters, must return (Ok or Err) and never panic.
    #[test]
    fn command_sweep_never_panics() {
        let hostile: Vec<Value> = vec![
            Value::Null,
            json!({}),
            json!([]),
            json!("string"),
            json!({"x": f64::MAX, "y": -f64::MAX, "width": 1e308, "height": 1e-308}),
            json!({"ids": [0, 1, 18446744073709551615u64], "dx": 1e300, "dy": -1e300}),
            json!({"width": 0, "height": 0, "radius": 0, "sides": 4294967295u64, "degrees": 1e308}),
            json!({"id": -1, "layer": 4294967295u64, "speed": "fast"}),
            json!({"path": "/nonexistent/\u{0}/x", "data": "\u{0}\u{1}\u{2}"}),
            json!({"commands": [{"id": "command.batch"}, {"id": "edit.delete"}]}),
        ];
        for spec in commands() {
            for params in &hostile {
                let mut s = Session::new();
                rect(&mut s);
                s.execute("shape.ellipse", &json!({"cx": 5, "cy": 5, "rx": 2, "ry": 2})).unwrap();
                let _ = s.execute(spec.id, params);
                // Invariants that must survive any input: layers intact, shapes reference real layers
                // and have finite geometry, and the planner still returns instead of panicking.
                assert_eq!(s.doc.layers.len(), 16, "{} with {params}", spec.id);
                for sh in &s.doc.shapes {
                    assert!(s.doc.layer(sh.layer).is_some(), "{} with {params}: dangling layer", spec.id);
                    let finite = sh.world_bounds().is_none_or(|b| b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite());
                    assert!(finite, "{} with {params}: non-finite geometry", spec.id);
                }
                let _ = s.execute("job.estimate", &Value::Null);
            }
        }
    }

    #[test]
    fn every_command_has_docs_and_a_unique_id() {
        let mut seen = std::collections::HashSet::new();
        for c in commands() {
            assert!(seen.insert(c.id), "duplicate id {}", c.id);
            assert!(!c.label.is_empty() && !c.params_doc.is_empty(), "{} lacks docs", c.id);
            assert!(find_command(c.id).is_some());
        }
        assert!(seen.len() >= 25, "{}", seen.len());
    }
}
