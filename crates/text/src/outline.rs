//! Outline text from a font file the person supplies at run time (TrueType or OpenType). No font
//! ships with the program or lives in the repository; the laid-out shapes are stored in the project,
//! so a project opens and cuts without the font, and only re-editing the text needs it again.
//!
//! Glyphs come from `skrifa`, shaping (kerning, ligatures, contextual forms) from `harfrust`, and
//! mixed left-to-right and right-to-left lines from the Unicode bidirectional algorithm
//! (`unicode-bidi`); all are MIT or Apache-2.0. A second font can supply characters the first lacks.

use kurbo::{Affine, BezPath};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider};

use crate::{Align, MAX_CHARS, MAX_HEIGHT_MM, MIN_HEIGHT_MM, TextError, TextLayout, TextStyle};

/// Largest font file read, in bytes.
pub const MAX_FONT_BYTES: usize = 64 * 1024 * 1024;
/// Cap height as a share of the em when the font does not say.
const FALLBACK_CAP: f64 = 0.7;

/// Lays out `text` with the outlines of the font in `data`, scaled so capitals are `style.height`
/// mm tall. The result is y down with the top of the first line's capitals at y = 0.
pub fn layout_outline(data: &[u8], text: &str, style: &TextStyle) -> Result<TextLayout, TextError> {
    layout_outline_with_fallback(data, None, text, style)
}

/// Like [`layout_outline`], but characters the main font has no glyph for are drawn from `fallback`
/// (if it has them), each font scaled to the same capital height. Only characters neither font has
/// are reported as unsupported.
pub fn layout_outline_with_fallback(data: &[u8], fallback: Option<&[u8]>, text: &str, style: &TextStyle) -> Result<TextLayout, TextError> {
    if !(style.height.is_finite() && (MIN_HEIGHT_MM..=MAX_HEIGHT_MM).contains(&style.height)) {
        return Err(TextError::BadHeight);
    }
    let ok = |v: f64| v.is_finite() && (0.0..=10.0).contains(&v);
    if !(ok(style.letter_spacing) && ok(style.line_spacing)) {
        return Err(TextError::BadSpacing);
    }
    if text.chars().count() > MAX_CHARS {
        return Err(TextError::TooLong);
    }
    let mut blobs = vec![data];
    blobs.extend(fallback);
    if blobs.iter().any(|b| b.len() > MAX_FONT_BYTES) {
        return Err(TextError::BadFont("the font file is too large".into()));
    }
    let fonts = blobs.iter().map(|b| FontRef::new(b).map_err(|e| TextError::BadFont(e.to_string()))).collect::<Result<Vec<_>, _>>()?;
    let loc = LocationRef::default();
    // Per font: scale to the requested cap height, and (for the first) the line height.
    let scales: Vec<f64> = fonts
        .iter()
        .map(|f| {
            let m = f.metrics(Size::unscaled(), loc);
            let cap = f64::from(m.cap_height.unwrap_or(0.0));
            let cap = if cap > 0.0 { cap } else { f64::from(m.units_per_em.max(1)) * FALLBACK_CAP };
            style.height / cap
        })
        .collect();
    let metrics = fonts[0].metrics(Size::unscaled(), loc);
    let scale = scales[0];
    let cap_mm = style.height;
    let line_height =
        ((f64::from(metrics.ascent) - f64::from(metrics.descent) + f64::from(metrics.leading)) * scale).max(cap_mm) * style.line_spacing;

    let outlines: Vec<_> = fonts.iter().map(|f| f.outline_glyphs()).collect();
    let charmaps: Vec<_> = fonts.iter().map(|f| f.charmap()).collect();
    let shaping_fonts = blobs
        .iter()
        .map(|b| harfrust::Font::new(b.to_vec(), 0).ok_or_else(|| TextError::BadFont("the font cannot be shaped".into())))
        .collect::<Result<Vec<_>, _>>()?;
    let shapers: Vec<_> = shaping_fonts.iter().map(harfrust::ShaperFont::new).collect();

    let mut path = BezPath::new();
    let mut unsupported: Vec<char> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut widths = Vec::with_capacity(lines.len());
    // (font index, glyph, x, y offset) per glyph, with positions in mm.
    let mut placed: Vec<Vec<(usize, skrifa::GlyphId, f64, f64)>> = Vec::with_capacity(lines.len());
    for line in &lines {
        let mut x = 0.0;
        let mut row = Vec::new();
        for (font_ix, run, rtl) in visual_pieces(line, &charmaps) {
            let (Some(shaper), Some(&sc)) = (shapers.get(font_ix), scales.get(font_ix)) else { continue };
            let mut buffer = harfrust::Buffer::new();
            buffer.push_str(run);
            buffer.set_direction(if rtl { harfrust::Direction::RightToLeft } else { harfrust::Direction::LeftToRight });
            buffer.guess_segment_properties();
            if harfrust::shape(shaper, &mut buffer, harfrust::ShapeOptions::new()).is_err() {
                continue;
            }
            for (info, pos) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
                if info.glyph_id == 0
                    && let Some(ch) = run.get(info.cluster as usize..).and_then(|rest| rest.chars().next())
                    && !ch.is_whitespace()
                    && !unsupported.contains(&ch)
                {
                    unsupported.push(ch);
                }
                let advance = f64::from(pos.x_advance) * sc * style.letter_spacing;
                row.push((font_ix, skrifa::GlyphId::new(info.glyph_id), x + f64::from(pos.x_offset) * sc, f64::from(pos.y_offset) * sc));
                x += advance;
            }
        }
        widths.push(x);
        placed.push(row);
    }
    let widest = widths.iter().copied().fold(0.0, f64::max);
    for (i, row) in placed.iter().enumerate() {
        let shift = match style.align {
            Align::Left => 0.0,
            Align::Center => (widest - widths[i]) / 2.0,
            Align::Right => widest - widths[i],
        };
        let baseline = style.height + i as f64 * line_height;
        for (font_ix, gid, x, y_off) in row {
            let (Some(glyphs), Some(&sc)) = (outlines.get(*font_ix), scales.get(*font_ix)) else { continue };
            let Some(glyph) = glyphs.get(*gid) else { continue };
            let mut pen = PathPen::default();
            if glyph.draw(DrawSettings::unhinted(Size::unscaled(), loc), &mut pen).is_err() {
                continue;
            }
            // Font units are y up; flip, scale and move to the pen position.
            let m = Affine::translate((x + shift, baseline - y_off)) * Affine::scale_non_uniform(sc, -sc);
            let mut g = pen.path;
            g.apply_affine(m);
            path.extend(g);
        }
    }
    Ok(TextLayout { path, unsupported })
}

/// A line as pieces in the order they are drawn, left to right: the Unicode bidirectional algorithm
/// (UAX #9) splits it into runs of one direction and puts them in visual order, and each run is split
/// again where the font changes (reversed within a right-to-left run). Each piece also says whether it
/// reads right to left, so it can be shaped that way.
fn visual_pieces<'a>(line: &'a str, charmaps: &[skrifa::charmap::Charmap<'_>]) -> Vec<(usize, &'a str, bool)> {
    let info = unicode_bidi::BidiInfo::new(line, None);
    let mut out = Vec::new();
    for para in &info.paragraphs {
        let (levels, level_runs) = info.visual_runs(para, para.range.clone());
        for range in level_runs {
            let Some(rtl) = levels.get(range.start).map(|l| l.is_rtl()) else { continue };
            let Some(text) = line.get(range) else { continue };
            let mut pieces: Vec<(usize, &str, bool)> = runs(text, charmaps).into_iter().map(|(f, s)| (f, s, rtl)).collect();
            if rtl {
                pieces.reverse();
            }
            out.extend(pieces);
        }
    }
    out
}

/// Splits a line into runs that each use one font: a character goes to the first font that maps it,
/// to the main font when none does, and whitespace stays with the run before it.
fn runs<'a>(line: &'a str, charmaps: &[skrifa::charmap::Charmap<'_>]) -> Vec<(usize, &'a str)> {
    let mut out: Vec<(usize, &str)> = Vec::new();
    let mut start = 0;
    let mut current = 0;
    for (at, ch) in line.char_indices() {
        let wanted = if ch.is_whitespace() { current } else { charmaps.iter().position(|c| c.map(ch).is_some()).unwrap_or(0) };
        if wanted != current && at > start {
            out.push((current, &line[start..at]));
            start = at;
        }
        current = wanted;
    }
    if start < line.len() {
        out.push((current, &line[start..]));
    }
    out
}

#[derive(Default)]
struct PathPen {
    path: BezPath,
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path.quad_to((f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to((f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y)));
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

#[cfg(any(test, feature = "testfont"))]
pub mod testfont;

#[cfg(test)]
mod tests;
