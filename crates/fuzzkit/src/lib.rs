//! Entry points for fuzzing every place untrusted bytes enter the program. Each function takes
//! arbitrary bytes and must return without panicking or hanging; errors are the expected outcome.
//!
//! Two drivers call them: the `cargo-fuzz` targets in `/fuzz` (coverage-guided, needs nightly), and
//! the deterministic mutation tests below, which run with `cargo test` on stable.

use lightfoundry_engine::{Session, commands};
use serde_json::Value;

/// Untrusted SVG text.
pub fn svg(data: &[u8]) {
    let _ = lightfoundry_svg::import_svg(data);
}

/// Untrusted DXF text.
pub fn dxf(data: &[u8]) {
    let _ = lightfoundry_dxf::import_dxf(data);
}

/// An untrusted `.lightfoundry` file.
pub fn project(data: &[u8]) {
    let _ = lightfoundry_format::from_bytes(data);
}

/// Lines a controller (or something pretending to be one) sends back.
pub fn controller_line(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    for line in text.lines() {
        let _ = lightfoundry_device::parse_status(line);
        let _ = lightfoundry_device::settings::parse_setting(line);
    }
}

/// A G-code program offered to the streamer.
pub fn gcode_program(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let _ = lightfoundry_device::jog_to_line(f64::from(data.first().copied().unwrap_or(0)) * 1e3, 0.0, 1000.0);
    let mut sim = lightfoundry_device::SimulatedGrbl::new();
    let _ = lightfoundry_device::stream_program(
        &mut sim,
        &text,
        &lightfoundry_device::StreamOptions::default(),
        &std::sync::atomic::AtomicBool::new(false),
        |_| {},
    );
}

/// A message on the MCP channel.
pub fn mcp_message(data: &[u8]) {
    let mut session = Session::new();
    let text = String::from_utf8_lossy(data);
    for line in text.lines() {
        let _ = lightfoundry_mcp::handle_line(&mut session, line);
    }
}

/// Text to lay out in the stroke font.
pub fn text_layout(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let _ = lightfoundry_text::layout(&text, &lightfoundry_text::TextStyle::default());
}

/// A font library glyph font: the first two bytes say where `font.json` ends and the glyph file starts.
/// Parsed, laid out in several styles, and registered with the engine and used through a text command.
pub fn glyph_font(data: &[u8]) {
    let Some((head, rest)) = data.split_at_checked(2) else { return };
    let at = (usize::from(head[0]) << 8 | usize::from(head[1])) % (rest.len() + 1);
    let (info, glyphs) = rest.split_at(at);
    let (info, glyphs) = (String::from_utf8_lossy(info), String::from_utf8_lossy(glyphs));
    if let Ok(font) = lightfoundry_text::GlyphFont::parse(&info, &glyphs) {
        let styles = [
            lightfoundry_text::TextStyle::default(),
            lightfoundry_text::TextStyle { height: 0.1, ..Default::default() },
            lightfoundry_text::TextStyle { height: 1000.0, letter_spacing: 10.0, line_spacing: 10.0, ..Default::default() },
            lightfoundry_text::TextStyle { align: lightfoundry_text::Align::Right, ..Default::default() },
        ];
        for style in &styles {
            let _ = lightfoundry_text::layout_glyph_font(&font, "Hello World\nÄ? abc 123", style);
        }
    }
    if lightfoundry_engine::fontlib::register("fuzz", &info, &glyphs).is_ok() {
        let mut session = Session::new();
        let _ = session.execute("text.create", &serde_json::json!({"text": "Hi there", "height": 10, "font": "library:fuzz"}));
    }
}

/// An Ink/Stitch font: the first two bytes say where `font.json` ends and the drawing starts.
pub fn inkstitch_font(data: &[u8]) {
    let Some((head, rest)) = data.split_at_checked(2) else { return };
    let at = (usize::from(head[0]) << 8 | usize::from(head[1])) % (rest.len() + 1);
    let (info, svg) = rest.split_at(at);
    let (info, svg) = (String::from_utf8_lossy(info), String::from_utf8_lossy(svg));
    if let Ok(font) = lightfoundry_text::GlyphFont::parse_inkstitch(&info, &svg) {
        for style in [
            lightfoundry_text::TextStyle::default(),
            lightfoundry_text::TextStyle { height: 1000.0, letter_spacing: 10.0, line_spacing: 10.0, ..Default::default() },
        ] {
            let _ = lightfoundry_text::layout_glyph_font(&font, "Hello World\nÄ? abc 123", &style);
        }
    }
}

/// The hosted site's font index and the list of fonts remembered from an earlier visit.
pub fn font_index(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let _ = lightfoundry_web::fontindex::parse_index(&text);
    let _ = lightfoundry_web::fontindex::parse_remembered(&text);
}

/// A repository tree listing from the network, and a path taken from it.
pub fn font_listing(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let _ = lightfoundry_fontlib::plan(&text, lightfoundry_fontlib::Set::Foundry);
    let _ = lightfoundry_fontlib::plan(&text, lightfoundry_fontlib::Set::Everything);
    let _ = lightfoundry_fontlib::safe_path(&text);
}

/// Replies from the AI service and the account service, in every parser: the first byte picks the HTTP
/// status and whether the request went through the account service, the rest is the body.
pub fn ai_reply(data: &[u8]) {
    let Some((first, rest)) = data.split_first() else { return };
    let text = String::from_utf8_lossy(rest);
    let status = [200u16, 200, 200, 401, 402, 429, 500, 503, 529, 404][usize::from(*first >> 1) % 10];
    let session = first & 1 == 1;
    let _ = lightfoundry_ai::art::parse_reply(status, &text, session);
    let _ = lightfoundry_ai::ask::parse_reply(status, &text, session);
    let _ = lightfoundry_ai::check::parse_reply(status, &text, session);
    let _ = lightfoundry_ai::check::read_check(&text);
    let _ = lightfoundry_ai::art::extract_svg(&text);
    let _ = lightfoundry_ai::cloud::clean_server(&text);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        let _ = lightfoundry_ai::cloud::session(&v);
        let _ = lightfoundry_ai::cloud::read_account(&v);
        let _ = lightfoundry_ai::cloud::checkout_url(&v);
    }
    let _ = lightfoundry_ai::cloud::parse(status, &text);
}

/// A command and its parameters: the first byte picks the command, the rest is JSON.
pub fn command(data: &[u8]) {
    let Some((first, rest)) = data.split_first() else { return };
    let specs: Vec<_> = commands().collect();
    let Some(spec) = specs.get(usize::from(*first) % specs.len().max(1)) else { return };
    // Commands that touch the file system are exercised by their own tests, not blindly.
    if spec.id.starts_with("file.") {
        return;
    }
    let params: Value = serde_json::from_slice(rest).unwrap_or(Value::Null);
    let mut session = Session::new();
    // Give the command something to work on.
    let _ = session.execute("shape.rectangle", &serde_json::json!({"x": 10, "y": 10, "width": 20, "height": 10}));
    let _ = session.execute(spec.id, &params);
    let _ = session.execute("job.gcode", &Value::Null);
    let _ = session.execute("edit.undo", &Value::Null);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so a failing input can be reproduced from its seed.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="50mm" height="30mm" viewBox="0 0 50 30"><g transform="translate(5 5)"><rect x="1" y="1" width="20" height="10" rx="2"/><circle cx="30" cy="10" r="6"/><path d="M0 0 C 5 20 15 20 20 0 Z"/></g></svg>"##;
    const DXF: &str = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n10\n0\nCIRCLE\n8\n0\n10\n5\n20\n5\n40\n3\n0\nLWPOLYLINE\n8\n0\n90\n3\n70\n1\n10\n0\n20\n0\n10\n4\n20\n0\n10\n4\n20\n4\n0\nENDSEC\n0\nEOF\n";
    /// Blocks, inserts, an ellipse and a spline: the parts of the DXF reader with the most arithmetic.
    const DXF_RICH: &str = "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nB\n10\n0\n20\n0\n0\nLINE\n10\n0\n20\n0\n11\n5\n21\n5\n0\nINSERT\n2\nB\n0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n2\nB\n10\n3\n20\n4\n41\n2\n42\n2\n50\n30\n0\nELLIPSE\n10\n0\n20\n0\n11\n9\n21\n1\n40\n0.4\n41\n0\n42\n3\n0\nSPLINE\n70\n8\n71\n3\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n41\n1\n41\n2\n41\n1\n41\n1\n10\n0\n20\n0\n10\n1\n20\n3\n10\n2\n20\n3\n10\n3\n20\n0\n0\nENDSEC\n0\nEOF\n";
    const GCODE: &str = "G21\nG90\nM4 S0\nG0 X10 Y10\nG1 X20 Y10 S500 F1000\nG1 X20 Y20\nM5\nG0 X0 Y0\n";
    const STATUS: &str = "<Idle|MPos:1.000,2.000,0.000|FS:0,0>\n$130=410.000\n$32=1\nok\nerror:20\nALARM:1\n";
    const MCP: &str = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"shape_rectangle\",\"arguments\":{\"width\":5,\"height\":5}}}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";

    /// Random bytes, plus the seed with a few bytes flipped, dropped, duplicated or truncated.
    fn mutations(seed: &[u8], rng: &mut Rng, count: usize) -> Vec<Vec<u8>> {
        let mut out = vec![Vec::new(), seed.to_vec()];
        for _ in 0..count {
            let mut v = seed.to_vec();
            for _ in 0..=rng.below(6) {
                if v.is_empty() {
                    break;
                }
                let i = rng.below(v.len());
                match rng.below(5) {
                    0 => v[i] = rng.next() as u8,
                    1 => {
                        v.remove(i);
                    }
                    2 => {
                        let b = v[i];
                        v.insert(i, b);
                    }
                    3 => v.truncate(i),
                    _ => {
                        let n = rng.below(40);
                        let chunk: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                        v.splice(i..i, chunk);
                    }
                }
            }
            out.push(v);
        }
        for _ in 0..count / 4 {
            let n = rng.below(300);
            out.push((0..n).map(|_| rng.next() as u8).collect());
        }
        out
    }

    fn drive(seed: &str, f: fn(&[u8]), count: usize) {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for input in mutations(seed.as_bytes(), &mut rng, count) {
            f(&input);
        }
    }

    #[test]
    fn svg_survives_mutation() {
        drive(SVG, svg, 1500);
    }

    #[test]
    fn dxf_survives_mutation() {
        drive(DXF, dxf, 3000);
        drive(DXF_RICH, dxf, 4000);
    }

    #[test]
    fn project_files_survive_mutation() {
        let mut s = Session::new();
        s.execute("shape.rectangle", &serde_json::json!({"width": 10, "height": 5})).unwrap();
        s.execute("preset.save", &serde_json::json!({"name": "p", "layer": 0})).unwrap();
        let bytes = lightfoundry_format::to_bytes(&s.doc).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        drive(&text, project, 1500);
    }

    #[test]
    fn controller_lines_and_programs_survive_mutation() {
        drive(STATUS, controller_line, 3000);
        drive(GCODE, gcode_program, 1500);
    }

    #[test]
    fn mcp_messages_survive_mutation() {
        drive(MCP, mcp_message, 1500);
    }

    /// Two glyph files in the real shape (anchors with handles, nested layers), behind the 2-byte split.
    fn glyph_font_seed() -> Vec<u8> {
        let info = r#"{"name":"Seed","horiz_adv_x":{"H":50,"o":40},"horiz_adv_x_space":30,"leading":98,"kerning_pairs":{}}"#;
        let glyphs = r#"{"format":"x","document":{"layers":[{"kind":{"type":"layer","children":[
            {"name":"H","kind":{"type":"path","path":{"subpaths":[{"anchors":[{"p":[0,0]},{"p":[0,-70]},{"p":[50,-70],"in":[40,-70],"out":[60,-70]},{"p":[50,0]}],"closed":true}]}}},
            {"name":"o","kind":{"type":"path","path":{"subpaths":[{"anchors":[{"p":[0,0]},{"p":[0,-50]},{"p":[40,-50]},{"p":[40,0]}],"closed":true},{"anchors":[{"p":[10,-10]},{"p":[10,-40]},{"p":[30,-40]}],"closed":true}]}}}]}}]}}"#;
        let mut data = vec![0, info.len() as u8];
        data.extend_from_slice(info.as_bytes());
        data.extend_from_slice(glyphs.as_bytes());
        data
    }

    #[test]
    fn glyph_fonts_survive_mutation() {
        let seed = glyph_font_seed();
        let mut rng = Rng(0xfeed_f00d);
        for input in mutations(&seed, &mut rng, 4000) {
            glyph_font(&input);
        }
        // The seed itself is a working font.
        assert!(
            lightfoundry_text::GlyphFont::parse(
                &String::from_utf8_lossy(&seed[2..2 + seed[1] as usize]),
                &String::from_utf8_lossy(&seed[2 + seed[1] as usize..])
            )
            .is_ok()
        );
    }

    #[test]
    fn inkstitch_fonts_survive_mutation() {
        let info = r#"{"name":"Seed","horiz_adv_x":{"H":14},"horiz_adv_x_space":9,"leading":30}"#;
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns:inkstitch="http://inkstitch.org/namespace" viewBox="0 0 30 30"><g transform="translate(1,2) scale(1.5)"><g inkscape:label="GlyphLayer-H" inkscape:groupmode="layer"><path inkstitch:satin_column="True" d="M 2 6 C 2 10 2 20 2 25 M 8 6 L 8 25 M 2 15 L 8 15"/><path d="M 8 15 Q 10 14 12 15"/></g><g inkscape:label="GlyphLayer-o"><path style="display:none" d="M 0 0 L 9 9"/><path d="M 1 12 L 9 12 A 4 4 0 0 1 9 25 Z" transform="rotate(5 3 3)"/></g></g></svg>"#;
        let mut seed = vec![0, info.len() as u8];
        seed.extend_from_slice(info.as_bytes());
        seed.extend_from_slice(svg.as_bytes());
        let mut rng = Rng(0xdec0_ded5);
        for input in mutations(&seed, &mut rng, 4000) {
            inkstitch_font(&input);
        }
        assert!(lightfoundry_text::GlyphFont::parse_inkstitch(info, svg).is_ok(), "the seed is a working font");
    }

    #[test]
    fn font_indexes_and_listings_survive_mutation() {
        let index = r#"{"fonts":[{"folder":"foundry-block","name":"Foundry Block","files":[{"path":"font.json","size":1917},{"path":"glyphs.stitchfoundry","size":176384}]}]}"#;
        drive(index, font_index, 2500);
        drive(r#"["foundry-block","foundry-sans","../x"]"#, font_index, 1500);
        let tree = r#"{"tree":[{"path":"fonts/a/font.json","type":"blob","size":10},{"path":"fonts/a/preview.png","type":"blob","size":9},{"path":"inkstitch/b/ltr.svg","type":"blob","size":99},{"path":"fonts","type":"tree"}],"truncated":false}"#;
        drive(tree, font_listing, 3000);
    }

    #[test]
    fn ai_replies_survive_mutation() {
        let message = r##"{"model":"m","stop_reason":"end_turn","content":[{"type":"text","text":"{\"summary\":\"ok\",\"suggestions\":[{\"layer\":1,\"speed\":900,\"passes\":2}]} <svg viewBox=\"0 0 1 1\"></svg>"}]}"##;
        let account =
            r#"{"token":"sfs_x","email":"a@b.c","credits":5,"payments":true,"url":"https://x.y/z","packs":[{"id":"s","credits":1,"cents":2}]}"#;
        for (first, seed) in [(0u8, message), (1, message), (7, message), (2, account), (9, account)] {
            let mut rng = Rng(0xa1_0000 + u64::from(first));
            for mut input in mutations(seed.as_bytes(), &mut rng, 1500) {
                input.insert(0, first);
                ai_reply(&input);
            }
        }
    }

    #[test]
    fn text_and_commands_survive_mutation() {
        drive("Hello, wörld!\nLine 2 \u{1F600} ~}{", text_layout, 1500);
        let mut rng = Rng(42);
        for _ in 0..3000 {
            let n = rng.below(120);
            let mut data: Vec<u8> = vec![rng.next() as u8];
            let junk = [
                "{\"id\":1}",
                "{\"width\":-5,\"height\":1e300}",
                "{\"x\":null}",
                "{\"ids\":[1,2,3],\"to\":\"left\"}",
                "[1,2]",
                "{\"text\":\"a\",\"height\":0}",
            ];
            data.extend_from_slice(junk[rng.below(junk.len())].as_bytes());
            data.extend((0..n % 8).map(|_| rng.next() as u8));
            command(&data);
        }
    }
}
