//! Lazy TTF metrics and individual outlines; never preprocess an entire font.
use geometry::lyon_tessellation::{
    FillRule,
    math::point,
    path::{Path, path::Builder},
};
use luma_vector_geometry::{self as geometry, Vertex};
use owned_ttf_parser::{AsFaceRef, GlyphId, OutlineBuilder, OwnedFace};

/// Curve flattening tolerance in em units: 0.15 physical px at 256 px/em, so the largest text Luma
/// draws (Title 1, lifted 10% on focus, on a 2x drawable) stays within about a tenth of a
/// physical pixel. The tolerance scales with the font size, never with the display density.
pub const FLATTENING_EM: f32 = 0.15 / 256.;

pub struct Font {
    face: OwnedFace,
}

pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
}

pub struct Metrics {
    pub advance_width: f32,
}

impl Font {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        OwnedFace::from_vec(bytes, 0)
            .map(|face| Self { face })
            .map_err(|e| e.to_string())
    }

    pub fn lookup_glyph_index(&self, character: char) -> u16 {
        self.face
            .as_face_ref()
            .glyph_index(character)
            .map_or(0, |id| id.0)
    }

    pub fn horizontal_line_metrics(&self, size: f32) -> Option<LineMetrics> {
        let face = self.face.as_face_ref();
        let scale = size / f32::from(face.units_per_em());
        Some(LineMetrics {
            ascent: f32::from(face.ascender()) * scale,
            descent: f32::from(face.descender()) * scale,
        })
    }

    pub fn metrics(&self, character: char, size: f32) -> Metrics {
        let face = self.face.as_face_ref();
        let glyph = GlyphId(self.lookup_glyph_index(character));
        Metrics {
            advance_width: f32::from(face.glyph_hor_advance(glyph).unwrap_or(0)) * size
                / f32::from(face.units_per_em()),
        }
    }

    pub fn horizontal_kern(&self, left: char, right: char, size: f32) -> Option<f32> {
        let face = self.face.as_face_ref();
        let left = face.glyph_index(left)?;
        let right = face.glyph_index(right)?;
        for table in face.tables().kern?.subtables {
            if table.horizontal
                && !table.has_cross_stream
                && let Some(value) = table.glyphs_kerning(left, right)
            {
                return Some(f32::from(value) * size / f32::from(face.units_per_em()));
            }
        }
        None
    }

    pub fn geometry(&self, character: char) -> Result<Vec<Vertex>, String> {
        match self.outline(character) {
            Some(path) => geometry::tessellate(&path, FLATTENING_EM, FillRule::NonZero),
            None => Ok(Vec::new()),
        }
    }

    /// A glyph's outline in em units (y down), or `None` for glyphs without one (space).
    pub fn outline(&self, character: char) -> Option<Path> {
        let face = self.face.as_face_ref();
        let mut outline = GlyphOutline {
            builder: Path::builder(),
            scale: 1. / f32::from(face.units_per_em()),
            open: false,
        };
        let glyph = GlyphId(self.lookup_glyph_index(character));
        face.outline_glyph(glyph, &mut outline)?;
        if outline.open {
            outline.builder.end(true);
        }
        Some(outline.builder.build())
    }
}

struct GlyphOutline {
    builder: Builder,
    scale: f32,
    open: bool,
}

impl OutlineBuilder for GlyphOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.builder.end(true);
        }
        self.builder.begin(point(x * self.scale, -y * self.scale));
        self.open = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(point(x * self.scale, -y * self.scale));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quadratic_bezier_to(
            point(x1 * self.scale, -y1 * self.scale),
            point(x * self.scale, -y * self.scale),
        );
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_bezier_to(
            point(x1 * self.scale, -y1 * self.scale),
            point(x2 * self.scale, -y2 * self.scale),
            point(x * self.scale, -y * self.scale),
        );
    }
    fn close(&mut self) {
        if self.open {
            self.builder.end(true);
            self.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn curve_flattening_stays_a_fraction_of_a_physical_pixel_at_every_size_we_draw() {
        // Title 1 (76 px) lifted 10% on focus, Larger text does not scale titles, at 1x and 2x.
        for density in [1., 2.] {
            let largest = 76. * 1.1 * density;
            let error = super::FLATTENING_EM * largest;
            assert!(error <= 0.1, "{error} px at density {density}");
        }
        // Body text with Larger text (29 px x 1.15) on the TV's 1x drawable.
        const { assert!(super::FLATTENING_EM * 29. * 1.15 <= 0.02) };
    }

    #[test]
    fn title_glyphs_have_no_seams_spikes_or_holes_at_density_1() {
        // The generated Inter instances exist after `pnpm assets` (CI runs tests before it;
        // the synthetic overlapping-contour test in vector-geometry always runs).
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/.build/assets/font-display-650.ttf"
        );
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("Skipped: {path} is generated by pnpm assets");
            return;
        };
        let font = super::Font::from_bytes(bytes).unwrap();
        // The greeting: Title 2, lifted 10% on focus, at density 1 with the one-pixel ramp.
        let size = 57. * 1.1;
        for character in ['n', 'm', 'e', 'G', 'o', 'a', 's', 'g'] {
            let outline = font.outline(character).unwrap();
            let mesh = font.geometry(character).unwrap();
            // Nothing outside the letter, and nothing missing inside it (a seam or crack where
            // overlapping contours meet), beyond the ramp's reach.
            let report = luma_vector_geometry::fringe_report(&outline, &mesh, size, 1., 0.75);
            assert!(
                report.stray < 0.05,
                "{character}: stray coverage {report:?}"
            );
            assert!(report.hole > 0.85, "{character}: hole inside {report:?}");
            // Per pixel, within the error a straight-edged glyph already has against exact
            // box-filtered coverage, including the light-text coverage boost.
            for exponent in [1., crate::renderer::LIGHT_TEXT_COVERAGE_EXPONENT] {
                let (excess, shortfall, at) = luma_vector_geometry::coverage_error(
                    &outline,
                    &mesh,
                    size,
                    1.,
                    exponent,
                    [10.3, 80.7],
                );
                assert!(excess < 0.35, "{character}: {excess} too much at {at:?}");
                assert!(shortfall < 0.25, "{character}: {shortfall} too little");
            }
        }
    }
}
