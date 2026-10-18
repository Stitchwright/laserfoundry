//! Design from Prompt and Redraw Image: what the model is asked to draw, and how its reply becomes artwork.
//!
//! The artwork comes back as an SVG in millimetres whose colours say what the laser does, using the
//! default layer colours: red outlines are cut through, blue lines are scored, black filled shapes are
//! engraved. `file.import_svg` (with `layer_by_color`) puts each colour on its layer.

use serde_json::{Value, json};

use super::{AiError, Auth, HttpRequest, Picture, body, check_pictures, clean_text, read_reply, send};

/// The largest artwork accepted, in bytes.
pub const MAX_SVG: usize = 1_000_000;
/// The narrowest part or gap the laser can be trusted with, in millimetres.
pub const MIN_FEATURE_MM: f64 = 1.5;
/// The range of sizes (longest side, mm) a request may ask for.
pub const SIZE_RANGE: (f64, f64) = (10.0, 1000.0);

/// What the artwork is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Use {
    /// Cut shapes out and engrave detail on them.
    #[default]
    CutAndEngrave,
    /// Engraving only: filled areas, nothing cut through.
    Engrave,
    /// Cutting only: outlines, nothing engraved.
    Cut,
}

/// Artwork to ask for.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtRequest {
    /// What to draw, in the person's words (notes on what to change, when redrawing a picture).
    pub prompt: String,
    /// The size of the longest side, in millimetres.
    pub size_mm: f64,
    pub use_for: Use,
    /// A picture to redraw.
    pub image: Option<Picture>,
}

/// Artwork the model drew.
#[derive(Clone, Debug, PartialEq)]
pub struct Art {
    /// The SVG document.
    pub svg: String,
    /// The model that drew it.
    pub model: String,
}

impl ArtRequest {
    /// The request with its size brought into range and the description trimmed.
    pub fn new(prompt: &str, size_mm: f64, use_for: Use) -> Self {
        let size_mm = if size_mm.is_finite() { size_mm.clamp(SIZE_RANGE.0, SIZE_RANGE.1) } else { 100.0 };
        Self { prompt: prompt.trim().to_string(), size_mm, use_for, image: None }
    }

    /// The request redrawing `image`.
    pub fn with_image(mut self, image: Picture) -> Self {
        self.image = Some(image);
        self
    }

    /// The instructions that make the model draw artwork a laser can use.
    pub fn system_prompt(&self) -> String {
        let size = self.size_mm.round();
        let min = MIN_FEATURE_MM;
        let colours = match self.use_for {
            Use::CutAndEngrave => {
                "- Outlines that are cut all the way through: closed paths with stroke=\"#ff0000\" and fill=\"none\".\n\
- Engraved detail: shapes filled solid black, fill=\"#000000\", with no stroke.\n\
- Light score lines that do not cut through: open or closed paths with stroke=\"#0000ff\" and fill=\"none\"."
            }
            Use::Engrave => "- Everything is engraved: shapes filled solid black, fill=\"#000000\", with no stroke. Nothing is cut through.",
            Use::Cut => "- Everything is cut through: closed paths with stroke=\"#ff0000\" and fill=\"none\". Nothing is engraved.",
        };
        let reply = "Reply with one SVG document and nothing else: no explanation and no Markdown code fence.";
        let units = format!(
            "- The SVG uses millimetres: width and height in mm (the longest side {size} mm) and a viewBox of the \
same numbers starting at 0 0. Absolute coordinates; no transforms on the root element."
        );
        let rules = format!(
            "- Only path, line, polyline, polygon, rect, circle and ellipse elements. No text, images, gradients, \
patterns, filters, masks, clip paths, opacity or fonts: draw letters as paths.\n\
- Strokes have stroke-width=\"0.1\". No part and no gap narrower than {min} mm: thinner features burn away or \
fall out. Where a cut-out shape has islands (the inside of an O), keep them joined to the rest by a bridge at \
least {min} mm wide.\n\
- Draw every outline once: no duplicated or overlapping paths, no tiny stray marks. Close the paths that are \
meant to be shapes. About 80 elements or fewer."
        );
        let refusal = "Never reproduce a logo, trademark, mascot or character that belongs to someone else (a team, company, \
brand, film, show or game). If asked for one, reply with a single line that starts with \"DECLINED:\" followed by a \
short reason, instead of the SVG.";
        if self.image.is_some() {
            return format!(
                "You redraw pictures as artwork for a laser engraver and cutter. {reply}\n\
\n\
The attached picture is the design. It will be about {size} mm across its longest side. Keep its composition, \
shapes, proportions and lettering as faithfully as a laser allows, and simplify only what a laser can't do \
(gradients and shading become solid areas or are left out):\n\
{colours}\n\
{units}\n\
{rules}\n\
- Leave out the picture's own background unless it is part of the design.\n\
- The artwork keeps the picture's aspect ratio and fills the viewBox."
            );
        }
        format!(
            "You draw artwork for a laser engraver and cutter. {reply}\n\
\n\
The artwork will be about {size} mm across its longest side. Colours say what the laser does:\n\
{colours}\n\
{units}\n\
{rules}\n\
- Bold, simple shapes with clear silhouettes, like a sticker or a sign: no shading, hatching or fine texture.\n\
\n\
{refusal}"
        )
    }

    /// The Messages API request body.
    pub fn body(&self) -> Value {
        let content = match &self.image {
            Some(img) => {
                let mut text = "Redraw this picture as laser artwork.".to_string();
                if !self.prompt.is_empty() {
                    text.push_str("\n\nWhat to change: ");
                    text.push_str(&self.prompt);
                }
                json!([img.block(), { "type": "text", "text": text }])
            }
            None => json!(format!("Draw this as laser artwork: {}", self.prompt)),
        };
        body(&self.system_prompt(), content)
    }

    /// The HTTP request to send, paid for by `auth`.
    pub fn http(&self, auth: &Auth) -> Result<HttpRequest, AiError> {
        clean_text(&self.prompt)?;
        // A redrawing needs no description: its notes are optional.
        if self.prompt.is_empty() && self.image.is_none() {
            return Err(AiError::NoPrompt);
        }
        if let Some(img) = &self.image {
            check_pictures(std::slice::from_ref(img))?;
        }
        send(auth, &self.body())
    }
}

/// The artwork in a reply with HTTP status `status` and body `body`; `session` says the request went
/// through the account service.
pub fn parse_reply(status: u16, body: &str, session: bool) -> Result<Art, AiError> {
    let (text, model) = read_reply(status, body, AiError::NoArt, session)?;
    if let Some(reason) = text.trim().strip_prefix("DECLINED:") {
        let reason: String = reason.trim().chars().take(300).collect();
        return Err(AiError::NotOriginal(if reason.is_empty() { "describe an original design".into() } else { reason }));
    }
    let svg = extract_svg(&text).ok_or(AiError::NoArt)?;
    Ok(Art { svg: svg.to_string(), model })
}

/// The SVG document in `text`: from its first `<svg` to its last `</svg>` (prose or a code fence around
/// it is left out).
pub fn extract_svg(text: &str) -> Option<&str> {
    let start = text.find("<svg")?;
    let end = text.rfind("</svg>")? + "</svg>".len();
    let svg = text.get(start..end)?;
    (svg.len() <= MAX_SVG).then_some(svg)
}
