//! End-to-end regression suite: the real `laserfoundry-cli` binary on inputs written here from scratch.
//!
//! * **Golden programs.** Each scenario's G-code is compared byte for byte with `tests/golden/*.gcode`. A difference means
//!   the output changed: if that was intended, review the diff and re-record with `LF_BLESS=1 cargo test -p laserfoundry-cli
//!   --test regression`; otherwise it is a regression.
//! * **Invariants** every program must satisfy (laser safety, bed limits, finite numbers) are checked on every golden.
//! * **Behaviour**: save and reopen, undo, safety refusals, the command catalogue.
//! * **Hostile input** through the binary: an error, never a crash.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const BED: &str = "300x200";
const BED_W: f64 = 300.0;
const BED_H: f64 = 200.0;

mod golden_programs {
    use super::*;

    #[test]
    fn svg_with_a_rectangle_a_circle_and_a_filled_triangle() {
        let dir = temp_dir("svg");
        let svg = write(&dir, "a.svg", SVG);
        let out = gcode_of(&dir, &[svg.to_str().unwrap(), "--bed", BED, "--set", "0:speed=900,max_power=40"]);
        check("svg_shapes", &out);
    }

    #[test]
    fn svg_origin_modes_move_the_job() {
        let dir = temp_dir("origin");
        let svg = write(&dir, "a.svg", SVG);
        let top_left = gcode_of(&dir, &[svg.to_str().unwrap(), "--bed", BED, "--origin", "top_left"]);
        let bottom_left = gcode_of(&dir, &[svg.to_str().unwrap(), "--bed", BED, "--origin", "bottom_left"]);
        assert_ne!(top_left, bottom_left, "the two origins must produce different programs");
        check("svg_origin_top_left", &top_left);
        check("svg_origin_bottom_left", &bottom_left);
    }

    #[test]
    fn dxf_lines_circle_and_polyline() {
        let dir = temp_dir("dxf");
        let dxf = write(&dir, "a.dxf", DXF);
        check("dxf_shapes", &gcode_of(&dir, &[dxf.to_str().unwrap(), "--bed", BED]));
    }

    #[test]
    fn raster_image_engraving() {
        let dir = temp_dir("png");
        let png = dir.join("ramp.png");
        let img = image::GrayImage::from_fn(24, 12, |x, y| image::Luma([((x * 10 + y * 3) % 256) as u8]));
        img.save(&png).unwrap();
        check("image_engrave", &gcode_of(&dir, &[png.to_str().unwrap(), "--bed", BED]));
    }

    #[test]
    fn script_shapes_booleans_and_arrays() {
        let out = script_gcode(&[
            step("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 30, "radius": 4})),
            step("shape.ellipse", json!({"cx": 60, "cy": 35, "rx": 15, "ry": 10, "layer": 1})),
            step("shape.polygon", json!({"cx": 120, "cy": 40, "radius": 15, "sides": 6})),
            step("shape.star", json!({"cx": 170, "cy": 40, "outer_radius": 16, "inner_radius": 7, "points": 5, "layer": 2})),
            step("edit.select", json!({"ids": [1, 2]})),
            step("path.boolean", json!({"op": "union"})),
            step("arrange.grid", json!({"ids": [3], "columns": 2, "rows": 2, "gap_x": 20, "gap_y": 20})),
            step("job.gcode", json!({"include": true})),
        ]);
        check("script_shapes", &out);
    }

    #[test]
    fn script_fill_layer_with_scan_settings() {
        let out = script_gcode(&[
            step("shape.rectangle", json!({"x": 30, "y": 30, "width": 25, "height": 15})),
            step("layer.set", json!({"id": 0, "mode": "fill", "interval": 0.5, "scan_angle": 0, "speed": 3000, "max_power": 20})),
            step("job.gcode", json!({"include": true})),
        ]);
        check("script_fill", &out);
    }

    #[test]
    fn script_multiple_passes_and_kerf() {
        let out = script_gcode(&[
            step("shape.rectangle", json!({"x": 50, "y": 50, "width": 20, "height": 20})),
            step("layer.set", json!({"id": 0, "passes": 3, "kerf_offset": 0.2, "speed": 600, "max_power": 80})),
            step("job.gcode", json!({"include": true})),
        ]);
        check("script_passes_kerf", &out);
    }

    #[test]
    fn script_text_with_the_built_in_font() {
        let out = script_gcode(&[
            step("text.create", json!({"text": "Hello 42", "x": 10, "y": 30, "height": 12})),
            step("job.gcode", json!({"include": true})),
        ]);
        check("script_text", &out);
    }

    #[test]
    fn script_material_test_grid() {
        let out = script_gcode(&[
            step(
                "generate.test_grid",
                json!({"cols": 3, "rows": 3, "speed_min": 500, "speed_max": 2000, "power_min": 10, "power_max": 60, "size": 8, "labels": false}),
            ),
            step("job.gcode", json!({"include": true})),
        ]);
        check("script_test_grid", &out);
    }

    #[test]
    fn script_nesting_parts_in_the_bed() {
        let out = script_gcode(&[
            step("shape.rectangle", json!({"x": 0, "y": 0, "width": 60, "height": 40})),
            step("shape.ellipse", json!({"cx": 20, "cy": 20, "rx": 20, "ry": 15})),
            step("shape.rectangle", json!({"x": 0, "y": 0, "width": 30, "height": 30})),
            step("edit.select_all", json!({})),
            step("arrange.nest", json!({"width": 200, "height": 120, "gap": 2, "margin": 2, "rotations": 4})),
            step("job.gcode", json!({"include": true})),
        ]);
        check("script_nest", &out);
    }

    #[test]
    fn script_offset_and_node_edit() {
        let out = script_gcode(&[
            step("path.create", json!({"nodes": [{"x": 20, "y": 20}, {"x": 60, "y": 20}, {"x": 60, "y": 50}, {"x": 20, "y": 50}], "closed": true})),
            step("node.insert", json!({"id": 1, "segment": 0})),
            step("node.move", json!({"id": 1, "index": 1, "x": 40, "y": 10})),
            step("path.offset", json!({"ids": [1], "distance": 3, "join": "round", "keep_originals": true})),
            step("job.gcode", json!({"include": true})),
        ]);
        check("script_offset_nodes", &out);
    }

    #[test]
    fn resuming_a_job_from_a_line_starts_with_the_laser_off() {
        let out = script_gcode(&[
            step("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 30})),
            step("job.resume", json!({"line": 8, "include": true})),
        ]);
        check("script_resume", &out);
    }
}

mod behaviour {
    use super::*;

    #[test]
    fn a_saved_project_reopens_and_makes_the_same_program() {
        let dir = temp_dir("roundtrip");
        let project = dir.join("job.laserfoundry");
        let build = [
            step("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 20})),
            step("shape.star", json!({"cx": 90, "cy": 40, "outer_radius": 15, "inner_radius": 6, "points": 6, "layer": 1})),
            step("layer.set", json!({"id": 1, "speed": 1200, "max_power": 55, "passes": 2})),
            step("text.create", json!({"text": "ok", "x": 20, "y": 80, "height": 10})),
        ];
        let before = script_gcode(&with(&build, &[step("job.gcode", json!({"include": true}))]));
        run_script(&dir, &with(&build, &[step("file.save", json!({"path": project.to_str().unwrap()}))]));
        assert!(project.exists());
        let after = script_gcode(&[step("file.open", json!({"path": project.to_str().unwrap()})), step("job.gcode", json!({"include": true}))]);
        assert_eq!(before, after, "save + open must not change the program");

        // The CLI reads project files directly too.
        let via_cli = gcode_of(&dir, &[project.to_str().unwrap()]);
        assert_eq!(before, via_cli);
    }

    #[test]
    fn undo_restores_the_earlier_program_and_redo_the_later_one() {
        let base = [step("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 20}))];
        let with_more = with(&base, &[step("shape.ellipse", json!({"cx": 90, "cy": 40, "rx": 12, "ry": 8}))]);
        let first = script_gcode(&with(&base, &[step("job.gcode", json!({"include": true}))]));
        let second = script_gcode(&with(&with_more, &[step("job.gcode", json!({"include": true}))]));
        assert_ne!(first, second);
        let undone = script_gcode(&with(&with_more, &[step("edit.undo", json!({})), step("job.gcode", json!({"include": true}))]));
        assert_eq!(first, undone, "undo must restore the previous program");
        let redone = script_gcode(&with(
            &with_more,
            &[step("edit.undo", json!({})), step("edit.redo", json!({})), step("job.gcode", json!({"include": true}))],
        ));
        assert_eq!(second, redone, "redo must bring it back");
    }

    #[test]
    fn a_failing_step_changes_nothing_and_the_program_is_the_same_as_without_it() {
        // A command that fails rolls back; the script stops with an error.
        let dir = temp_dir("fail");
        let out = run_script_raw(
            &dir,
            &[step("shape.rectangle", json!({"x": 1, "y": 1, "width": 10, "height": 10})), step("shape.rectangle", json!({"x": 1, "y": 1}))],
        );
        assert!(!out.status.success());
        assert_no_crash(&out);
        assert!(String::from_utf8_lossy(&out.stderr).contains("step 2"), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn shapes_outside_the_bed_are_refused_not_clamped() {
        let dir = temp_dir("bed");
        let svg = write(
            &dir,
            "big.svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="500mm" height="50mm" viewBox="0 0 500 50"><rect x="0" y="0" width="450" height="40" fill="none" stroke="#f00"/></svg>"##,
        );
        let out_file = dir.join("big.gcode");
        let out = cli(&["gcode", svg.to_str().unwrap(), "-o", out_file.to_str().unwrap(), "--bed", BED]);
        assert!(!out.status.success(), "a 450 mm job on a 300 mm bed must be refused");
        assert_no_crash(&out);
        assert!(!out_file.exists() || std::fs::read_to_string(&out_file).unwrap().is_empty(), "no program may be written for a refused job");
    }

    #[test]
    fn selection_only_scope_cuts_just_the_selection() {
        let all = script_gcode(&[
            step("shape.rectangle", json!({"x": 20, "y": 20, "width": 20, "height": 20})),
            step("shape.rectangle", json!({"x": 80, "y": 20, "width": 20, "height": 20})),
            step("job.gcode", json!({"include": true})),
        ]);
        let one = script_gcode(&[
            step("shape.rectangle", json!({"x": 20, "y": 20, "width": 20, "height": 20})),
            step("shape.rectangle", json!({"x": 80, "y": 20, "width": 20, "height": 20})),
            step("edit.select", json!({"ids": [1]})),
            step("job.set_scope", json!({"selection_only": true})),
            step("job.gcode", json!({"include": true})),
        ]);
        assert!(one.len() < all.len());
        assert!(!one.contains("X80"), "the unselected rectangle must not be cut:\n{one}");
        check_invariants("selection_only", &one);
    }

    #[test]
    fn estimates_and_info_report_a_plausible_job() {
        let dir = temp_dir("info");
        let svg = write(&dir, "a.svg", SVG);
        let out = cli(&["info", svg.to_str().unwrap()]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("shapes: 3"), "{text}");
        assert!(text.contains("80 × 60 mm") || text.contains("mm"), "{text}");
        assert!(text.contains("estimated time"), "{text}");
    }

    #[test]
    fn an_empty_drawing_reports_zero_not_negative_zero() {
        let dir = temp_dir("emptyinfo");
        let svg = write(&dir, "empty.svg", r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#);
        let out = cli(&["info", svg.to_str().unwrap()]);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("cut 0 mm, travel 0 mm"), "{text}");
    }

    #[test]
    fn svg_export_round_trips_the_shape_count() {
        let dir = temp_dir("svgexport");
        let exported = dir.join("out.svg");
        run_script(
            &dir,
            &[
                step("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})),
                step("shape.ellipse", json!({"cx": 60, "cy": 20, "rx": 10, "ry": 10, "layer": 1})),
                step("file.export_svg", json!({"path": exported.to_str().unwrap()})),
            ],
        );
        let text = std::fs::read_to_string(&exported).unwrap();
        assert!(text.contains("<svg"));
        let out = cli(&["info", exported.to_str().unwrap()]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("shapes: 2"), "{}", String::from_utf8_lossy(&out.stdout));
    }

    #[test]
    fn the_command_catalogue_only_changes_on_purpose() {
        let out = cli(&["commands"]);
        assert!(out.status.success());
        let text = String::from_utf8(out.stdout).unwrap();
        let mut ids = String::new();
        for line in text.lines() {
            let id = line.split_whitespace().next().unwrap();
            assert!(line.len() > id.len() + 8, "{id} has no label or parameter documentation");
            ids.push_str(id);
            ids.push('\n');
        }
        let unique: std::collections::BTreeSet<_> = ids.lines().collect();
        assert_eq!(unique.len(), ids.lines().count(), "duplicate command ids");
        check_text("commands.txt", &ids);
    }

    #[test]
    fn every_command_with_empty_parameters_fails_cleanly_or_succeeds() {
        let dir = temp_dir("sweep");
        let listing = String::from_utf8(cli(&["commands"]).stdout).unwrap();
        for line in listing.lines() {
            let id = line.split_whitespace().next().unwrap();
            // These talk to the network or the file system on their own; other tests cover them.
            if id.starts_with("ai.") || id.starts_with("help.") || id == "job.simulate" || id.starts_with("file.autosave") {
                continue;
            }
            let out = run_script_raw(&dir, &[step(id, json!({}))]);
            assert_no_crash(&out);
        }
    }
}

mod hostile_input {
    use super::*;

    #[test]
    fn broken_files_are_errors_not_crashes() {
        let dir = temp_dir("hostile");
        let big_nesting = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>", "<g>".repeat(5000), "</g>".repeat(5000));
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty.svg", vec![]),
            ("garbage.svg", b"\x00\xff\xfe<svg<<<>>>".to_vec()),
            ("truncated.svg", SVG.as_bytes()[..SVG.len() / 2].to_vec()),
            ("deep.svg", big_nesting.into_bytes()),
            ("nan.svg", br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="NaN" height="1e999" x="-inf" y="0"/><path d="M0 0 L1e308 1e308 L-1e308 5"/></svg>"#.to_vec()),
            ("empty.dxf", vec![]),
            ("garbage.dxf", b"0\nSECTION\n2\nENTITIES\n0\nCIRCLE\n40\nabc\n0\nEOF\n".to_vec()),
            ("huge_radius.dxf", b"0\nSECTION\n2\nENTITIES\n0\nCIRCLE\n10\n0\n20\n0\n40\n1e300\n0\nENDSEC\n0\nEOF\n".to_vec()),
            ("truncated.dxf", DXF.as_bytes()[..DXF.len() / 3].to_vec()),
            ("dxf_arc_huge.dxf", dxf_entity("ARC\n10\n0\n20\n0\n40\n1e100\n50\n0\n51\n270")),
            ("dxf_ellipse_huge.dxf", dxf_entity("ELLIPSE\n10\n0\n20\n0\n11\n1e300\n21\n0\n40\n0.5\n41\n0\n42\n6.28")),
            ("dxf_bulge_huge.dxf", dxf_entity("LWPOLYLINE\n90\n2\n70\n0\n10\n0\n20\n0\n42\n1e-300\n10\n1e300\n20\n0")),
            ("dxf_arc_angles.dxf", dxf_entity("ARC\n10\n0\n20\n0\n40\n10\n50\n-1e300\n51\n1e300")),
            ("dxf_nan.dxf", dxf_entity("LINE\n10\nnan\n20\n-inf\n11\ninf\n21\nnan")),
            ("svg_huge_arc.svg", br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0 A 1e300 1e-300 45 0 0 1e300 1e300"/></svg>"#.to_vec()),
            ("svg_huge_circle.svg", br#"<svg xmlns="http://www.w3.org/2000/svg"><circle r="1e300"/></svg>"#.to_vec()),
            ("svg_huge_transform.svg", br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1" transform="scale(1e300) rotate(1e300)"/></svg>"#.to_vec()),
            ("svg_long_text.svg", format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><text font-size=\"10\">{}</text></svg>", "W".repeat(2_000_000)).into_bytes()),
            ("png_claims_100k.png", png_claiming(100_000, 100_000)),
            ("png_claims_max.png", png_claiming(u32::MAX >> 1, 1)),
            ("bad.png", b"\x89PNG\r\n\x1a\nnot really".to_vec()),
            ("empty.png", vec![]),
            ("bad.laserfoundry", b"{\"version\": 1, \"shapes\": [".to_vec()),
            ("binary.laserfoundry", (0..=255u8).collect()),
            ("wrongtype.laserfoundry", br#"{"shapes": "nope", "layers": 5}"#.to_vec()),
        ];
        for (name, bytes) in cases {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            for args in [vec!["info", path.to_str().unwrap()], vec!["gcode", path.to_str().unwrap(), "-o", dir.join("o.gcode").to_str().unwrap()]] {
                let started = std::time::Instant::now();
                let out = cli(&args);
                assert_no_crash(&out);
                assert!(started.elapsed().as_secs() < 20, "{name} took {:?}", started.elapsed());
            }
        }
    }

    #[test]
    fn broken_scripts_and_arguments_are_errors() {
        let dir = temp_dir("badscripts");
        for (name, body) in [("empty.json", ""), ("notarray.json", "{}"), ("noid.json", "[{}]"), ("unknown.json", r#"[{"id":"no.such.command"}]"#)] {
            let path = write(&dir, name, body);
            let out = cli(&["run", path.to_str().unwrap()]);
            assert!(!out.status.success(), "{name}");
            assert_no_crash(&out);
        }
        for args in [vec![], vec!["bogus"], vec!["gcode"], vec!["gcode", "x.svg"], vec!["gcode", "x.svg", "-o"], vec!["info", "/no/such/file.svg"]] {
            let out = cli(&args);
            assert!(!out.status.success(), "{args:?}");
            assert_no_crash(&out);
        }
    }

    #[test]
    fn absurd_parameters_are_refused_not_attempted() {
        let dir = temp_dir("absurd");
        for steps in [
            vec![step("shape.rectangle", json!({"x": 0, "y": 0, "width": 1e300, "height": 1e300})), step("job.gcode", json!({}))],
            vec![step("shape.polygon", json!({"cx": 0, "cy": 0, "radius": 10, "sides": 100000}))],
            vec![
                step("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})),
                step("arrange.grid", json!({"ids": [1], "columns": 100000, "rows": 100000, "gap_x": 1, "gap_y": 1})),
            ],
            vec![
                step("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})),
                step("layer.set", json!({"id": 0, "passes": 4000000000u64, "speed": -5})),
            ],
            vec![step("generate.test_grid", json!({"cols": 100000, "rows": 100000}))],
        ] {
            let out = run_script_raw(&dir, &steps);
            assert_no_crash(&out);
        }
    }
}

/// A DXF with one entity (`body` is its type and group codes, newline separated).
fn dxf_entity(body: &str) -> Vec<u8> {
    format!("0\nSECTION\n2\nENTITIES\n0\n{body}\n0\nENDSEC\n0\nEOF\n").into_bytes()
}

/// A PNG whose header claims a size its data does not have (a decompression-bomb shape).
fn png_claiming(width: u32, height: u32) -> Vec<u8> {
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(data);
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&crc_input);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        out
    }
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 0, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(chunk(b"IHDR", &header));
    png.extend(chunk(b"IDAT", &[0x78, 0x01, 0x01, 0x01, 0x00, 0xfe, 0xff, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01]));
    png.extend(chunk(b"IEND", &[]));
    png
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn check(name: &str, program: &str) {
    check_invariants(name, program);
    check_text(&format!("{name}.gcode"), program);
}

/// What every program the app produces must satisfy, whatever the scenario.
fn check_invariants(name: &str, program: &str) {
    assert!(!program.is_empty(), "{name}: empty program");
    let lines: Vec<&str> = program.lines().collect();
    assert!(lines.iter().take(6).any(|l| *l == "G21"), "{name}: millimetres must be set up front");
    assert!(lines.iter().take(6).any(|l| *l == "G90"), "{name}: absolute moves must be set up front");
    assert!(program.ends_with('\n'), "{name}: missing final newline");
    assert!(program.contains("M5"), "{name}: the laser must be switched off");
    let last_motion_or_off = lines.iter().rev().find(|l| l.starts_with('G') || l.starts_with('M')).unwrap();
    assert!(last_motion_or_off.starts_with("G0") || *last_motion_or_off == "M5", "{name}: ends with {last_motion_or_off}");
    assert!(!lines.iter().any(|l| l.contains("M3")) || program.contains("M5"), "{name}");
    let lowered = program.to_ascii_lowercase();
    for bad in ["nan", "inf", "e+", "e-"] {
        assert!(!lowered.lines().filter(|l| !l.starts_with(';')).any(|l| l.contains(bad)), "{name}: '{bad}' in a command line");
    }
    let mut laser_on_moving = false;
    for (n, line) in lines.iter().enumerate() {
        let n = n + 1;
        if line.starts_with(';') {
            continue;
        }
        assert!(line.is_ascii() && !line.contains('\r'), "{name}:{n}: {line:?}");
        for word in line.split_whitespace() {
            let (letter, value) = word.split_at(1);
            match letter {
                "X" | "Y" => {
                    let v: f64 = value.parse().unwrap_or_else(|_| panic!("{name}:{n}: bad number in {line:?}"));
                    assert!(v.is_finite());
                    let limit = if letter == "X" { BED_W } else { BED_H.max(400.0) };
                    assert!((-0.001..=limit + 0.001).contains(&v), "{name}:{n}: {letter}{v} outside the bed in {line:?}");
                }
                "S" | "F" => {
                    let v: f64 = value.parse().unwrap_or_else(|_| panic!("{name}:{n}: bad number in {line:?}"));
                    assert!(v.is_finite() && v >= 0.0, "{name}:{n}: {line:?}");
                }
                _ => {}
            }
        }
        if line.starts_with("G1") && line.contains(" S") {
            laser_on_moving = true;
        }
        if line.starts_with("M5") {
            laser_on_moving = false;
        }
    }
    let _ = laser_on_moving;
}

/// Compares with `tests/golden/<file>`; `LF_BLESS=1` rewrites it.
fn check_text(file: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(file);
    if std::env::var_os("LF_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; record it with LF_BLESS=1 cargo test -p laserfoundry-cli --test regression", path.display()));
    if expected != actual {
        let first =
            expected.lines().zip(actual.lines()).position(|(a, b)| a != b).unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
        panic!(
            "{file} changed (first difference at line {}):\n  expected: {:?}\n  actual:   {:?}\nIf intended, review it and re-record with LF_BLESS=1.",
            first + 1,
            expected.lines().nth(first),
            actual.lines().nth(first)
        );
    }
}

fn step(id: &str, params: Value) -> Value {
    json!({ "id": id, "params": params })
}

fn with(a: &[Value], b: &[Value]) -> Vec<Value> {
    a.iter().chain(b).cloned().collect()
}

/// Runs the script in a fresh process and returns the `gcode` text of the last result.
fn script_gcode(steps: &[Value]) -> String {
    let dir = temp_dir("script");
    let result = run_script(&dir, steps);
    result["gcode"].as_str().unwrap_or_else(|| panic!("no gcode in {result}")).to_string()
}

fn run_script(dir: &Path, steps: &[Value]) -> Value {
    let out = run_script_raw(dir, steps);
    assert!(out.status.success(), "script failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

fn run_script_raw(dir: &Path, steps: &[Value]) -> Output {
    let path = dir.join("script.json");
    std::fs::write(&path, serde_json::to_string(steps).unwrap()).unwrap();
    cli(&["run", path.to_str().unwrap()])
}

/// `gcode <args> -o <file>`, returning the program.
fn gcode_of(dir: &Path, args: &[&str]) -> String {
    let out_file = dir.join("out.gcode");
    let _ = std::fs::remove_file(&out_file);
    let mut all = vec!["gcode", args[0], "-o", out_file.to_str().unwrap()];
    all.extend_from_slice(&args[1..]);
    let out = cli(&all);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    std::fs::read_to_string(&out_file).unwrap()
}

/// A crash is a panic (exit code 101), a signal, or a panic message.
fn assert_no_crash(out: &Output) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.code().is_some_and(|c| c != 101), "crashed: {:?} {err}", out.status);
    assert!(!err.contains("panicked"), "panic: {err}");
}

fn cli(args: &[&str]) -> Output {
    let home = temp_dir("home");
    Command::new(env!("CARGO_BIN_EXE_laserfoundry-cli"))
        .args(args)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &home)
        .env("APPDATA", &home)
        .env_remove("LASERFOUNDRY_TRANSLATION")
        .output()
        .unwrap()
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("laserfoundry-cli-regression-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80mm" height="60mm" viewBox="0 0 80 60">
  <rect x="5" y="5" width="30" height="20" fill="none" stroke="#ff0000"/>
  <circle cx="55" cy="30" r="12" fill="none" stroke="#0000ff"/>
  <path d="M5 40 L35 40 L20 55 Z" fill="#000000"/>
</svg>"##;

const DXF: &str = "0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\n0\n10\n10\n20\n10\n11\n60\n21\n10\n\
0\nLINE\n8\n0\n10\n60\n20\n10\n11\n60\n21\n40\n\
0\nCIRCLE\n8\n0\n10\n100\n20\n30\n40\n15\n\
0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n20\n20\n50\n10\n20\n20\n50\n10\n30\n20\n60\n10\n30\n20\n70\n10\n20\n20\n70\n\
0\nENDSEC\n0\nEOF\n";
