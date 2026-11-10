//! A small GLES2-compatible GPU painter. Rust owns the glyph atlas, vectors and the draw
//! list; SDL only supplies the context. No browser, LVGL or Qt runtime.
use crate::{
    font::Font,
    glyph_atlas::{self, Atlas, Bitmaps, GlyphKey, Slot},
    raster::{Size, Viewport},
    state::Rect,
    vector,
};
use gl::types::*;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

pub type Color = [f32; 4];
pub const WHITE: Color = [0.96, 0.96, 0.96, 1.];
/// Coverage exponent for light text. The GPU blends in sRGB, which thins light-on-dark edges;
/// raising coverage to this power (about 1/1.43) restores their perceived weight. Dark text
/// keeps linear coverage, and mid tones blend between the two (`glyph.vert`).
pub const LIGHT_TEXT_COVERAGE_EXPONENT: f32 = 0.7;
/// Laid-out text runs kept between frames (glyph positions only; bitmaps live in the atlas).
const TEXT_RUNS: usize = 512;
/// The glyph atlas: single-channel, starting at 1024² (1 MiB) and growing once to 2048².
const ATLAS_SIZE: u32 = 1024;
const ATLAS_MAX_SIZE: u32 = 2048;
/// How long an ordinary frame waits for glyphs still being rasterised before drawing without
/// them (they appear in a following frame); the first frame and captures wait longer.
const GLYPH_WAIT: std::time::Duration = std::time::Duration::from_millis(4);
const GLYPH_WAIT_FIRST: std::time::Duration = std::time::Duration::from_secs(3);
/// `GL_LUMINANCE`, GLES 2's single-channel texture format (absent from the core bindings).
const LUMINANCE: GLenum = 0x1909;
const TEXTURE_BUDGET: usize = 48 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub size: f32,
    pub weight: u16,
    /// CSS letter-spacing in em, not pixels.
    pub tracking: f32,
    pub line_height: f32,
    pub align_right: bool,
}
impl TextStyle {
    pub const fn new(size: f32) -> Self {
        Self {
            size,
            weight: 400,
            tracking: 0.,
            line_height: 1.2,
            align_right: false,
        }
    }
    pub const fn weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }
    pub const fn tracking(mut self, tracking: f32) -> Self {
        self.tracking = tracking;
        self
    }
    pub const fn leading(mut self, height: f32) -> Self {
        self.line_height = height;
        self
    }
    pub const fn right(mut self) -> Self {
        self.align_right = true;
        self
    }
}

#[derive(Clone)]
pub enum Draw {
    Shape(Rect, f32, f32, Color),
    RoundedGradient(Rect, f32, Color, Color),
    Shadow(Rect, f32, f32, Color),
    Text(String, Rect, TextStyle, Color, bool),
    Image(String, Rect, f32, bool),
    TintedImage(String, Rect, f32, bool, Color),
    /// Installed icon and its backdrop, composited before opacity and edge fading.
    AppImage(Arc<crate::app_artwork::Artwork>, Rect, f32, Color),
    /// A blurred wallpaper copy sampled at the same screen coordinates, clipped to a rounded
    /// rectangle: the base of a Liquid Glass panel.
    Backdrop(String, Rect, f32, Color),
    /// A rounded border of the given width whose colour runs from top to bottom: glass rims.
    Rim(Rect, f32, f32, Color, Color),
    Clip(Option<Rect>),
    EdgeFade(Option<[f32; 4]>),
}

struct Texture {
    id: GLuint,
    width: u32,
    height: u32,
    frame: u64,
}
impl Drop for Texture {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteTextures(1, &self.id);
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct TextKey {
    text: String,
    weight: u16,
    // Logical size, tracking, line height, available width and height.
    layout: [u32; 5],
    flags: u8,
}

/// A laid-out text run: each glyph's font, index and pen position relative to the run's box.
struct TextRun {
    glyphs: Vec<Placed>,
    size: f32,
    missing_glyphs: usize,
    frame: u64,
}

struct Placed {
    font: u16,
    glyph: u16,
    x: f32,
    baseline: f32,
}

#[derive(Clone, Serialize)]
pub struct GraphicsInfo {
    pub vendor: String,
    pub renderer: String,
    pub version: String,
    pub max_texture_size: u32,
    pub max_viewport: [i32; 2],
    pub fragment_highp: bool,
}

#[derive(Clone, Serialize)]
pub struct RenderStats {
    pub target: Size,
    pub viewport: Viewport,
    pub font_antialiasing: &'static str,
    pub multisample_samples: u32,
    pub max_logical_font_px: f32,
    pub max_physical_font_px: f32,
    /// Glyphs packed in the atlas, and its side in texels.
    pub cached_glyphs: usize,
    pub atlas_size: u32,
    /// Glyph bitmaps uploaded to the atlas this frame.
    pub glyph_uploads: usize,
    /// Glyphs this frame needed that the worker had not finished (drawn in a later frame).
    pub pending_glyphs: usize,
    /// Milliseconds the SDL thread spent this frame on glyph uploads and waiting for the
    /// rasteriser; the rasterisation itself runs on the glyph worker.
    pub glyph_ms: f64,
    pub vector_load_ms: f64,
    pub image_load_ms: f64,
    pub glyph_failures: usize,
    pub missing_glyphs: usize,
    pub missing_images: usize,
    /// CPU copies of rasterised glyphs, and the atlas texture's size.
    pub glyph_bitmap_bytes: usize,
    pub atlas_bytes: usize,
    pub cached_text_runs: usize,
    pub text_quads: usize,
    pub artwork_bytes: usize,
    pub vector_meshes: usize,
    pub vector_bytes: usize,
    pub vector_triangles: usize,
    pub high_resolution_wallpaper: bool,
    pub draw_calls: usize,
    pub cpu_submission_ms: f64,
    /// Share of the drawable redrawn in the last frame (1 without partial redraw).
    pub redrawn_fraction: f64,
}
impl RenderStats {
    fn new(target: Size) -> Self {
        Self {
            target,
            viewport: Viewport::new(target),
            font_antialiasing: "cpu glyph atlas, 256-level coverage",
            multisample_samples: 0,
            max_logical_font_px: 0.,
            max_physical_font_px: 0.,
            cached_glyphs: 0,
            atlas_size: 0,
            glyph_uploads: 0,
            pending_glyphs: 0,
            glyph_ms: 0.,
            vector_load_ms: 0.,
            image_load_ms: 0.,
            glyph_failures: 0,
            missing_glyphs: 0,
            missing_images: 0,
            glyph_bitmap_bytes: 0,
            atlas_bytes: 0,
            cached_text_runs: 0,
            text_quads: 0,
            artwork_bytes: 0,
            vector_meshes: 0,
            vector_bytes: 0,
            vector_triangles: 0,
            high_resolution_wallpaper: false,
            draw_calls: 0,
            cpu_submission_ms: 0.,
            redrawn_fraction: 1.,
        }
    }
}

enum BatchKind {
    Paint {
        texture: GLuint,
        start: usize,
        count: usize,
        backdrop: Option<[[u8; 3]; 2]>,
    },
    Vector {
        path: String,
        rect: Rect,
        tint: Color,
    },
    /// Glyph quads in `glyph_vertices`, all from the atlas.
    Glyphs { start: usize, count: usize },
}

struct Batch {
    kind: BatchKind,
    clip: Option<Rect>,
    // [top fade-in, top end, bottom start, bottom fade-out]; 0 = off.
    fade: Option<[f32; 4]>,
}

fn full_screen(rect: &Rect) -> bool {
    rect.x <= 0. && rect.y <= 0. && rect.x + rect.w >= 1920. && rect.y + rect.h >= 1080.
}

/// An opaque, untinted, square full-screen image: the wallpaper that scrims can fold into.
fn plain_wallpaper(command: &Draw) -> Option<&str> {
    match command {
        Draw::Image(path, rect, radius, false) if full_screen(rect) && *radius <= 0. => Some(path),
        Draw::TintedImage(path, rect, radius, false, tint)
            if full_screen(rect) && *radius <= 0. && *tint == [1.; 4] =>
        {
            Some(path)
        }
        _ => None,
    }
}

/// A full-screen translucent fill without corners or border: a flat colour or a vertical
/// gradient (straight alpha, interpolated like the paint shader's vertex colours).
enum Scrim {
    Flat(Color),
    Gradient(Color, Color),
}
impl Scrim {
    fn of(command: &Draw) -> Option<Self> {
        match command {
            Draw::Shape(rect, radius, border, color)
                if full_screen(rect) && *radius <= 0. && *border <= 0. =>
            {
                Some(Self::Flat(*color))
            }
            Draw::RoundedGradient(rect, radius, top, bottom)
                if full_screen(rect) && *radius <= 0. =>
            {
                Some(Self::Gradient(*top, *bottom))
            }
            _ => None,
        }
    }
    fn at(&self, t: f32) -> Color {
        match self {
            Self::Flat(color) => *color,
            Self::Gradient(top, bottom) => {
                std::array::from_fn(|i| top[i] + (bottom[i] - top[i]) * t)
            }
        }
    }
}

fn intersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let top = (a[1] + a[3]).min(b[1] + b[3]);
    [x, y, (right - x).max(0), (top - y).max(0)]
}

/// Square images skip the rounded-rectangle distance field: a negative radius selects the
/// shader's plain texture path. Per pixel that field cost as much as the rest of the Home scene
/// on a webOS 5 Mali for the full-screen wallpaper.
fn plain_radius(radius: f32) -> f32 {
    if radius > 0. { radius } else { -1. }
}

/// Measurement aid: LUMA_PROFILE_SKIP=wallpaper,image,app,shape,gradient,shadow,text or all
/// leaves those draws out, to find which part of the scene costs the GPU time.
fn profile_skip(command: &Draw) -> bool {
    static SKIP: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let skip = SKIP.get_or_init(|| {
        std::env::var("LUMA_PROFILE_SKIP")
            .map(|value| value.split(',').map(str::to_owned).collect())
            .unwrap_or_default()
    });
    if skip.is_empty() {
        return false;
    }
    let kind = match command {
        Draw::Image(_, rect, ..) | Draw::TintedImage(_, rect, ..)
            if rect.w >= 1920. && rect.h >= 1080. =>
        {
            "wallpaper"
        }
        Draw::Image(..) | Draw::TintedImage(..) | Draw::Backdrop(..) => "image",
        Draw::AppImage(..) => "app",
        Draw::Shape(..) => "shape",
        Draw::RoundedGradient(..) | Draw::Rim(..) => "gradient",
        Draw::Shadow(..) => "shadow",
        Draw::Text(..) => "text",
        Draw::Clip(_) | Draw::EdgeFade(_) => return false,
    };
    skip.iter().any(|entry| entry == kind || entry == "all")
}

fn snap_rect(rect: Rect, scale: f32) -> Rect {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var("LUMA_SNAP").as_deref() != Ok("0")) {
        return rect;
    }
    let scale = scale.max(0.01);
    let snap = |value: f32| (value * scale).round() / scale;
    let (x0, y0) = (snap(rect.x), snap(rect.y));
    // Never collapse a visible hairline.
    let x1 = snap(rect.x + rect.w).max(x0 + 1. / scale);
    let y1 = snap(rect.y + rect.h).max(y0 + 1. / scale);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

pub struct Renderer {
    program: GLuint,
    vector_program: GLuint,
    glyph_program: GLuint,
    buffer: GLuint,
    glyph_buffer: GLuint,
    vao: GLuint,
    white: Texture,
    font_directory: PathBuf,
    fonts: HashMap<u16, Font>,
    failed_fonts: HashSet<u16>,
    text_runs: HashMap<TextKey, TextRun>,
    glyph_worker: glyph_atlas::Worker,
    glyph_bitmaps: Bitmaps,
    atlas: Atlas,
    atlas_texture: Texture,
    gles: bool,
    /// The atlas filled up while building this frame: clear or grow it and build again.
    atlas_full: bool,
    /// Glyphs the last drawn frame lacked; their arrival needs a redraw.
    awaited: HashSet<GlyphKey>,
    glyph_vertices: Vec<f32>,
    /// Until the first frame is drawn, frames wait for every glyph (no text pops in).
    first_frame: bool,
    /// Rasterisation time reported by the worker, and glyphs, since the last reset.
    pub glyph_raster_ms: f64,
    pub glyphs_rasterised: usize,
    textures: HashMap<String, Option<Texture>>,
    vectors: HashMap<String, Option<vector::Mesh>>,
    artwork_variants: HashMap<String, String>,
    vertices: Vec<f32>,
    batches: Vec<Batch>,
    clip: Option<Rect>,
    // [top fade-in, top end, bottom start, bottom fade-out]; 0 = off.
    fade: Option<[f32; 4]>,
    frame: u64,
    stats: RenderStats,
    pub info: GraphicsInfo,
}

impl Renderer {
    pub fn new(font: &Path, gles: bool) -> Result<Self, String> {
        let directory = font.parent().ok_or("Invalid font path")?;
        let mut fonts = HashMap::new();
        // Open only the common face; the other weights and optical sizes stay lazy.
        fonts.insert(400, load_font(directory, 400)?);
        let mut max_texture_size = 0;
        let mut max_viewport = [0; 2];
        let mut precision = 0;
        unsafe {
            gl::GetIntegerv(gl::MAX_TEXTURE_SIZE, &mut max_texture_size);
            gl::GetIntegerv(gl::MAX_VIEWPORT_DIMS, max_viewport.as_mut_ptr());
            if gles && gl::GetShaderPrecisionFormat::is_loaded() {
                let mut range = [0; 2];
                gl::GetShaderPrecisionFormat(
                    gl::FRAGMENT_SHADER,
                    gl::HIGH_FLOAT,
                    range.as_mut_ptr(),
                    &mut precision,
                );
            }
        }
        let fragment_highp = !gles || precision > 0;
        let qualifier = if fragment_highp { "highp" } else { "mediump" };
        let vertex_prefix = if gles {
            format!("#version 100\nprecision highp float;\n#define PAINT_PRECISION {qualifier}\n")
        } else {
            "#version 150\n#define attribute in\n#define varying out\n#define PAINT_PRECISION\n"
                .into()
        };
        let fragment_prefix = if gles {
            format!(
                "#version 100\nprecision {qualifier} float;\n#define PAINT_PRECISION {qualifier}\n"
            )
        } else {
            "#version 150\n#define varying in\n#define texture2D texture\nout vec4 out_color;\n#define gl_FragColor out_color\n#define PAINT_PRECISION\n".into()
        };
        let program = link(
            &(vertex_prefix.clone() + include_str!("shaders/paint.vert")),
            &(fragment_prefix.clone() + include_str!("shaders/paint.frag")),
            &[c"a_position", c"a_uv", c"a_color", c"a_shape", c"a_params"],
        )?;
        let vector_program = match link(
            &(vertex_prefix.clone() + include_str!("shaders/vector.vert")),
            &(fragment_prefix.clone() + include_str!("shaders/vector.frag")),
            &[c"a_position", c"a_offset", c"a_color", c"a_coverage"],
        ) {
            Ok(program) => program,
            Err(error) => {
                unsafe { gl::DeleteProgram(program) };
                return Err(error);
            }
        };
        let glyph_program = match link(
            &(vertex_prefix + include_str!("shaders/glyph.vert")),
            &(fragment_prefix + include_str!("shaders/glyph.frag")),
            &[c"a_position", c"a_uv", c"a_color"],
        ) {
            Ok(program) => program,
            Err(error) => {
                unsafe {
                    gl::DeleteProgram(program);
                    gl::DeleteProgram(vector_program);
                }
                return Err(error);
            }
        };
        let mut buffer = 0;
        let mut glyph_buffer = 0;
        let mut vao = 0;
        unsafe {
            if !gles {
                gl::GenVertexArrays(1, &mut vao);
                gl::BindVertexArray(vao);
            }
            gl::GenBuffers(1, &mut buffer);
            gl::GenBuffers(1, &mut glyph_buffer);
            gl::Enable(gl::BLEND);
            // Preserve an opaque destination alpha. Using BlendFunc for both
            // channels makes translucent UI darken again in PNG/window compositing.
            gl::BlendFuncSeparate(
                gl::SRC_ALPHA,
                gl::ONE_MINUS_SRC_ALPHA,
                gl::ONE,
                gl::ONE_MINUS_SRC_ALPHA,
            );
            gl::Disable(gl::DEPTH_TEST);
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
        }
        let white = texture(1, 1, &[255; 4]);
        let max_atlas = (max_texture_size.max(0) as u32).clamp(ATLAS_SIZE, ATLAS_MAX_SIZE);
        Ok(Self {
            program,
            vector_program,
            glyph_program,
            buffer,
            glyph_buffer,
            vao,
            white,
            font_directory: directory.into(),
            fonts,
            failed_fonts: HashSet::new(),
            text_runs: HashMap::new(),
            glyph_worker: glyph_atlas::Worker::new(directory.into(), load_font),
            glyph_bitmaps: Bitmaps::default(),
            atlas: Atlas::new(ATLAS_SIZE, max_atlas),
            atlas_texture: atlas_texture(ATLAS_SIZE, gles),
            gles,
            atlas_full: false,
            awaited: HashSet::new(),
            glyph_vertices: Vec::with_capacity(16384),
            first_frame: true,
            glyph_raster_ms: 0.,
            glyphs_rasterised: 0,
            textures: HashMap::new(),
            vectors: HashMap::new(),
            artwork_variants: HashMap::new(),
            vertices: Vec::with_capacity(65536),
            batches: Vec::new(),
            clip: None,
            fade: None,
            frame: 0,
            stats: RenderStats::new(Size::HD),
            info: GraphicsInfo {
                vendor: gl_string(gl::VENDOR),
                renderer: gl_string(gl::RENDERER),
                version: gl_string(gl::VERSION),
                max_texture_size: max_texture_size.max(0) as u32,
                max_viewport,
                fragment_highp,
            },
        })
    }

    fn ensure_font(&mut self, id: u16) -> bool {
        if self.fonts.contains_key(&id) {
            return true;
        }
        if self.failed_fonts.contains(&id) {
            return false;
        }
        match load_font(&self.font_directory, id) {
            Ok(font) => {
                self.fonts.insert(id, font);
                true
            }
            Err(error) => {
                eprintln!("{error}");
                self.failed_fonts.insert(id);
                false
            }
        }
    }

    /// The atlas slot to draw a glyph from: packed already, packed now from its CPU copy, or,
    /// while the worker still rasterises it (requested here), another packed size of the same
    /// glyph to stand in scaled. `None` draws nothing yet.
    fn glyph_slot(&mut self, key: GlyphKey) -> Option<(GlyphKey, Slot)> {
        let frame = self.frame;
        if let Some(slot) = self.atlas.get(&key, frame) {
            return Some((key, slot));
        }
        if let Some(coverage) = self.glyph_bitmaps.get(&key, frame) {
            let Some(slot) = self.atlas.insert(key, &coverage, frame) else {
                self.atlas_full = true;
                return None;
            };
            self.upload_glyph(slot, &coverage);
            return Some((key, slot));
        }
        self.glyph_worker.request(key);
        self.awaited.insert(key);
        self.stats.pending_glyphs += 1;
        self.atlas.stand_in(&key, frame)
    }

    /// Uploads a glyph with a cleared one-texel border, so linear filtering at its edges never
    /// reads what an earlier packing left there.
    fn upload_glyph(&mut self, slot: Slot, coverage: &luma_vector_geometry::raster::Coverage) {
        if slot.width == 0 || slot.height == 0 {
            return;
        }
        let started = Instant::now();
        let (w, h) = (slot.width as usize, slot.height as usize);
        let mut padded = vec![0u8; (w + 2) * (h + 2)];
        for (row, line) in coverage.data.chunks_exact(w).enumerate() {
            padded[(row + 1) * (w + 2) + 1..][..w].copy_from_slice(line);
        }
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, self.atlas_texture.id);
            gl::TexSubImage2D(
                gl::TEXTURE_2D,
                0,
                slot.x as i32 - 1,
                slot.y as i32 - 1,
                w as i32 + 2,
                h as i32 + 2,
                atlas_format(self.gles),
                gl::UNSIGNED_BYTE,
                padded.as_ptr().cast(),
            );
        }
        self.stats.glyph_uploads += 1;
        self.stats.glyph_ms += started.elapsed().as_secs_f64() * 1000.;
    }

    /// Takes finished glyphs from the worker into the CPU cache. True when one that the last
    /// drawn frame lacked has arrived (that frame needs drawing again, in full).
    fn accept(&mut self, results: Vec<glyph_atlas::Rasterised>) -> bool {
        let mut needed = false;
        for result in results {
            self.glyph_raster_ms += result.ms;
            self.glyphs_rasterised += 1;
            needed |= self.awaited.remove(&result.key);
            self.glyph_bitmaps
                .insert(result.key, result.coverage, self.frame);
        }
        needed
    }

    /// Collects glyphs the worker has finished (call once per loop iteration).
    pub fn poll_glyphs(&mut self) -> bool {
        let results = self.glyph_worker.receive(None);
        self.accept(results)
    }

    /// Whether glyphs are still being rasterised (the loop should wake soon to poll).
    pub fn glyphs_pending(&self) -> bool {
        self.glyph_worker.pending() > 0
    }

    fn trim_text(&mut self) {
        if self.text_runs.len() > TEXT_RUNS {
            let mut old: Vec<_> = self
                .text_runs
                .iter()
                .filter(|(_, run)| run.frame != self.frame)
                .map(|(key, run)| (key.clone(), run.frame))
                .collect();
            old.sort_by_key(|(_, frame)| *frame);
            let excess = self.text_runs.len() - TEXT_RUNS;
            for (key, _) in old.into_iter().take(excess) {
                self.text_runs.remove(&key);
            }
        }
        self.glyph_bitmaps.trim(self.frame);
        self.stats.cached_glyphs = self.atlas.len();
        self.stats.atlas_size = self.atlas.packer.size;
        self.stats.atlas_bytes = (self.atlas.packer.size as usize).pow(2);
        self.stats.glyph_bitmap_bytes = self.glyph_bitmaps.bytes();
        self.stats.cached_text_runs = self.text_runs.len();
    }

    fn quad(
        &mut self,
        rect: Rect,
        uv: [f32; 4],
        radius: f32,
        border: f32,
        color: Color,
        texture: GLuint,
    ) {
        self.gradient_quad(rect, uv, radius, border, [color, color], texture, None);
    }

    #[allow(clippy::too_many_arguments)]
    fn gradient_quad(
        &mut self,
        rect: Rect,
        uv: [f32; 4],
        radius: f32,
        border: f32,
        colors: [Color; 2],
        texture: GLuint,
        backdrop: Option<[[u8; 3]; 2]>,
    ) {
        if rect.w <= 0. || rect.h <= 0. {
            return;
        }
        let rect = self.snap(rect);
        let start = self.vertices.len() / 14;
        for (x, y) in [(0., 0.), (1., 0.), (0., 1.), (0., 1.), (1., 0.), (1., 1.)] {
            let color = colors[y as usize];
            self.vertices.extend_from_slice(&[
                rect.x + x * rect.w,
                rect.y + y * rect.h,
                uv[0] + x * (uv[2] - uv[0]),
                uv[1] + y * (uv[3] - uv[1]),
                color[0],
                color[1],
                color[2],
                color[3],
                (x - 0.5) * rect.w,
                (y - 0.5) * rect.h,
                rect.w / 2.,
                rect.h / 2.,
                radius,
                border,
            ]);
        }
        self.paint_batch(start, 6, texture, backdrop);
    }

    /// Appends `count` paint vertices from `start`, merged into the previous batch when it uses
    /// the same texture, clip and fade.
    fn paint_batch(
        &mut self,
        start: usize,
        count: usize,
        texture: GLuint,
        backdrop: Option<[[u8; 3]; 2]>,
    ) {
        if let Some(batch) = self.batches.last_mut() {
            let same_clip = match (batch.clip, self.clip) {
                (None, None) => true,
                (Some(a), Some(b)) => a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h,
                _ => false,
            };
            if same_clip
                && batch.fade == self.fade
                && let BatchKind::Paint {
                    texture: previous,
                    count: batch_count,
                    backdrop: previous_backdrop,
                    ..
                } = &mut batch.kind
                && *previous == texture
                && *previous_backdrop == backdrop
            {
                *batch_count += count;
                return;
            }
        }
        self.batches.push(Batch {
            kind: BatchKind::Paint {
                texture,
                start,
                count,
                backdrop,
            },
            clip: self.clip,
            fade: self.fade,
        });
    }

    /// The wallpaper with full-screen translucent fills folded in: each fill maps a pixel p to
    /// p * (1 - a) + c * a, so together they are p * m + k, drawn in one pass instead of one
    /// full-screen layer each. Strips keep gradients' products of colour and alpha exact.
    fn folded_quad(&mut self, rect: Rect, texture: GLuint, scrims: &[Scrim]) {
        const STRIPS: usize = 16;
        let fold = |t: f32| {
            let (mut m, mut k) = ([1f32; 3], [0f32; 3]);
            for scrim in scrims {
                let color = scrim.at(t);
                for i in 0..3 {
                    m[i] *= 1. - color[3];
                    k[i] = k[i] * (1. - color[3]) + color[i] * color[3];
                }
            }
            (m, k)
        };
        let start = self.vertices.len() / 14;
        for strip in 0..STRIPS {
            let rows = [
                strip as f32 / STRIPS as f32,
                (strip + 1) as f32 / STRIPS as f32,
            ];
            let folds = rows.map(fold);
            for (x, row) in [(0., 0), (1., 0), (0., 1), (0., 1), (1., 0), (1., 1)] {
                let (m, k) = folds[row];
                let t = rows[row];
                self.vertices.extend_from_slice(&[
                    rect.x + x * rect.w,
                    rect.y + t * rect.h,
                    x,
                    t,
                    m[0],
                    m[1],
                    m[2],
                    1.,
                    k[0],
                    k[1],
                    k[2],
                    0.,
                    -2.,
                    0.,
                ]);
            }
        }
        self.paint_batch(start, STRIPS * 6, texture, None);
    }

    fn text(&mut self, text: &str, rect: Rect, style: TextStyle, color: Color, centered: bool) {
        // Layout is logical and cached per run; glyph bitmaps come from the atlas at the
        // drawable's physical size.
        let size_f = style.size;
        let viewport = self.stats.viewport;
        self.stats.max_logical_font_px = self.stats.max_logical_font_px.max(size_f);
        self.stats.max_physical_font_px = self
            .stats
            .max_physical_font_px
            .max(size_f * viewport.text_scale());
        let padded = rect.inset(-size_f);
        if padded.x + padded.w < 0.
            || padded.y + padded.h < 0.
            || padded.x > 1920.
            || padded.y > 1080.
            || self.clip.is_some_and(|clip| {
                padded.x + padded.w < clip.x
                    || padded.y + padded.h < clip.y
                    || padded.x > clip.x + clip.w
                    || padded.y > clip.y + clip.h
            })
        {
            return;
        }
        let key = TextKey {
            text: text.into(),
            weight: style.weight,
            layout: [
                size_f.to_bits(),
                style.tracking.to_bits(),
                style.line_height.to_bits(),
                rect.w.to_bits(),
                rect.h.to_bits(),
            ],
            flags: u8::from(style.align_right) | (u8::from(centered) << 1),
        };
        if !self.text_runs.contains_key(&key) {
            let Some(run) = self.layout(text, rect.w, rect.h, style, centered) else {
                return;
            };
            self.text_runs.insert(key.clone(), run);
        }
        let run = self.text_runs.get_mut(&key).unwrap();
        run.frame = self.frame;
        self.stats.missing_glyphs += run.missing_glyphs;
        // Glyph origins snap to physical pixels, so every bitmap lands 1:1 on the grid.
        let (sx, sy) = (
            viewport.width as f32 / 1920.,
            viewport.height as f32 / 1080.,
        );
        let physical = run.size * sy;
        let glyphs: Vec<_> = run
            .glyphs
            .iter()
            .map(|glyph| {
                (
                    GlyphKey::new(glyph.font, glyph.glyph, physical),
                    ((rect.x + glyph.x) * sx).round(),
                    ((rect.y + glyph.baseline) * sy).round(),
                )
            })
            .collect();
        let start = self.glyph_vertices.len() / 8;
        for (key, x, y) in glyphs {
            let Some((drawn, slot)) = self.glyph_slot(key) else {
                continue;
            };
            if slot.width == 0 || slot.height == 0 {
                continue;
            }
            // 1 except for a stand-in of another size, drawn scaled until this one arrives.
            let scale = key.px() / drawn.px();
            let x0 = (x + slot.left as f32 * scale) / sx;
            let y0 = (y + slot.top as f32 * scale) / sy;
            let x1 = x0 + slot.width as f32 * scale / sx;
            let y1 = y0 + slot.height as f32 * scale / sy;
            let texels = self.atlas.packer.size as f32;
            let (u0, v0) = (slot.x as f32 / texels, slot.y as f32 / texels);
            let (u1, v1) = (
                (slot.x + slot.width) as f32 / texels,
                (slot.y + slot.height) as f32 / texels,
            );
            for (px, py, u, v) in [
                (x0, y0, u0, v0),
                (x1, y0, u1, v0),
                (x0, y1, u0, v1),
                (x1, y0, u1, v0),
                (x1, y1, u1, v1),
                (x0, y1, u0, v1),
            ] {
                self.glyph_vertices
                    .extend_from_slice(&[px, py, u, v, color[0], color[1], color[2], color[3]]);
            }
            self.stats.text_quads += 1;
        }
        let count = self.glyph_vertices.len() / 8 - start;
        if count == 0 {
            return;
        }
        // Consecutive text with the same clip and fade is one draw call.
        if let Some(Batch {
            kind: BatchKind::Glyphs { start: s, count: c },
            clip,
            fade,
        }) = self.batches.last_mut()
            && *s + *c == start
            && same_clip(*clip, self.clip)
            && *fade == self.fade
        {
            *c += count;
            return;
        }
        self.batches.push(Batch {
            kind: BatchKind::Glyphs { start, count },
            clip: self.clip,
            fade: self.fade,
        });
    }

    /// Lays a text run out in logical units: line breaking, alignment and kerning.
    fn layout(
        &mut self,
        text: &str,
        width: f32,
        height: f32,
        style: TextStyle,
        centered: bool,
    ) -> Option<TextRun> {
        let size_f = style.size;
        let rect = Rect::new(0., 0., width, height);
        let font_id = style.weight + if style.size >= 32. { 1000 } else { 0 };
        if !self.ensure_font(font_id) {
            self.stats.glyph_failures += text.chars().count();
            return None;
        }
        // Only Inter ships; characters it lacks, such as CJK in an app title, draw as its
        // missing-glyph box rather than loading a fallback font.
        let fonts = &self.fonts;
        let font = &fonts[&font_id];
        let line_height = size_f * style.line_height;
        let baseline = font.horizontal_line_metrics(size_f).map_or(size_f, |m| {
            (line_height - m.ascent + m.descent) / 2. + m.ascent
        });
        let tracking = style.tracking * size_f;
        let advance = |c| {
            if c == '\n' {
                0.
            } else {
                font.metrics(c, size_f).advance_width + tracking
            }
        };
        let mut lines: Vec<String> = Vec::new();
        let mut line = String::new();
        let mut width = 0.;
        // Word wrapping, with character fallback for a title wider than the tile.
        for word in text.split_inclusive(char::is_whitespace) {
            let word_width: f32 = word.chars().map(advance).sum();
            if width > 0. && width + word_width > rect.w {
                lines.push(line.trim_end().into());
                line.clear();
                width = 0.;
            }
            for c in word.chars() {
                if c == '\n' {
                    lines.push(line.trim_end().into());
                    line.clear();
                    width = 0.;
                    continue;
                }
                let w = advance(c);
                if width > 0. && width + w > rect.w {
                    lines.push(line.trim_end().into());
                    line.clear();
                    width = 0.;
                }
                line.push(c);
                width += w;
            }
        }
        if !line.is_empty() {
            lines.push(line.trim_end().into());
        }
        let mut glyphs = Vec::new();
        let mut missing_glyphs = 0;
        for (index, line) in lines.iter().enumerate() {
            let top = rect.y + index as f32 * line_height;
            if index > 0 && top + line_height > rect.y + rect.h + 1. {
                break;
            }
            let mut previous = None;
            let width: f32 = line
                .chars()
                .map(|c| {
                    let kern = previous
                        .and_then(|p| font.horizontal_kern(p, c, size_f))
                        .unwrap_or(0.);
                    previous = Some(c);
                    advance(c) + kern
                })
                .sum();
            let mut x = rect.x
                + if style.align_right {
                    rect.w - width
                } else if centered {
                    (rect.w - width) / 2.
                } else {
                    0.
                };
            let mut previous = None;
            for c in line.chars() {
                if let Some(previous) = previous {
                    x += font.horizontal_kern(previous, c, size_f).unwrap_or(0.);
                }
                let glyph = font.lookup_glyph_index(c);
                if glyph == 0 && !c.is_whitespace() {
                    missing_glyphs += 1;
                }
                if !c.is_whitespace() {
                    glyphs.push(Placed {
                        font: font_id,
                        glyph,
                        x,
                        baseline: top + baseline,
                    });
                }
                x += advance(c);
                previous = Some(c);
            }
        }
        Some(TextRun {
            glyphs,
            size: size_f,
            missing_glyphs,
            frame: self.frame,
        })
    }

    fn image(&mut self, path: &str, rect: Rect, radius: f32, contain: bool, tint: Color) {
        self.folded_image(path, rect, radius, contain, tint, &[]);
    }

    fn folded_image(
        &mut self,
        path: &str,
        mut rect: Rect,
        radius: f32,
        contain: bool,
        tint: Color,
        scrims: &[Scrim],
    ) {
        if path.ends_with(".lvec") {
            self.vector_image(path, rect, contain, tint);
            return;
        }
        let path = if self.stats.viewport.scale > 1.
            && self.info.max_texture_size >= 3840
            && Path::new(path)
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|p| p == "wallpapers")
        {
            self.artwork_variants
                .entry(path.into())
                .or_insert_with(|| {
                    let original = Path::new(path);
                    let variant = original.with_file_name(format!(
                        "{}@2x.jpg",
                        original.file_stem().unwrap_or_default().to_string_lossy()
                    ));
                    if variant.is_file() {
                        variant.to_string_lossy().into_owned()
                    } else {
                        path.into()
                    }
                })
                .clone()
        } else {
            path.into()
        };
        if path.ends_with("@2x.jpg") {
            self.stats.high_resolution_wallpaper = true;
        }
        let Some((id, width, height)) = self.texture(&path) else {
            return;
        };
        if contain {
            let scale = (rect.w / width as f32).min(rect.h / height as f32);
            let w = width as f32 * scale;
            let h = height as f32 * scale;
            rect = Rect::new(rect.x + (rect.w - w) / 2., rect.y + (rect.h - h) / 2., w, h);
        }
        if scrims.is_empty() {
            self.quad(rect, [0., 0., 1., 1.], plain_radius(radius), 0., tint, id);
        } else {
            self.folded_quad(rect, id, scrims);
        }
    }

    /// Samples a blurred wallpaper at the screen position it lies over, so translucent glass
    /// shows the (static) background through it without a real-time blur pass.
    fn backdrop(&mut self, path: &str, rect: Rect, radius: f32, tint: Color) {
        let Some((id, ..)) = self.texture(path) else {
            return;
        };
        let uv = [
            rect.x / 1920.,
            rect.y / 1080.,
            (rect.x + rect.w) / 1920.,
            (rect.y + rect.h) / 1080.,
        ];
        self.quad(rect, uv, plain_radius(radius), 0., tint, id);
    }

    /// Loads (once) and marks a raster texture as used this frame.
    fn texture(&mut self, path: &str) -> Option<(GLuint, u32, u32)> {
        let path = path.to_owned();
        let frame = self.frame;
        let started = Instant::now();
        let loaded = !self.textures.contains_key(&path);
        let entry = self
            .textures
            .entry(path.clone())
            .or_insert_with(|| match image::open(&path) {
                Ok(image) => {
                    let image = image.into_rgba8();
                    Some(texture(image.width(), image.height(), image.as_raw()))
                }
                Err(error) => {
                    eprintln!("Artwork {path}: {error}");
                    None
                }
            });
        if loaded {
            self.stats.image_load_ms += started.elapsed().as_secs_f64() * 1000.;
        }
        let Some(texture) = entry else {
            self.stats.missing_images += 1;
            return None;
        };
        texture.frame = frame;
        Some((texture.id, texture.width, texture.height))
    }

    /// Edges on whole drawable pixels. The TV scales its 1080p graphics plane up 2x, which
    /// turns a half-covered edge pixel into a soft 2-pixel band; whole pixels stay crisp.
    /// LUMA_SNAP=0 keeps logical positions (for comparison).
    fn snap(&self, rect: Rect) -> Rect {
        snap_rect(rect, self.stats.viewport.scale)
    }

    fn vector_image(&mut self, path: &str, mut rect: Rect, contain: bool, tint: Color) {
        if rect.w <= 0. || rect.h <= 0. {
            return;
        }
        let scale = self.stats.viewport.scale;
        let started = Instant::now();
        let loaded = !self.vectors.contains_key(path);
        let mesh = self.vectors.entry(path.into()).or_insert_with(|| {
            match vector::Mesh::load(Path::new(path)) {
                Ok(mesh) => Some(mesh),
                Err(error) => {
                    eprintln!("{error}");
                    None
                }
            }
        });
        if loaded {
            self.stats.vector_load_ms += started.elapsed().as_secs_f64() * 1000.;
        }
        let Some(mesh) = mesh else {
            self.stats.missing_images += 1;
            return;
        };
        if contain {
            let scale = (rect.w / mesh.width).min(rect.h / mesh.height);
            let width = mesh.width * scale;
            let height = mesh.height * scale;
            rect = Rect::new(
                rect.x + (rect.w - width) / 2.,
                rect.y + (rect.h - height) / 2.,
                width,
                height,
            );
        }
        // Logos keep their size; only their placement moves onto the pixel grid.
        let snapped = snap_rect(rect, scale);
        rect = Rect::new(snapped.x, snapped.y, rect.w, rect.h);
        let clip = if let Some(clip) = self.clip {
            let x = rect.x.max(clip.x);
            let y = rect.y.max(clip.y);
            Rect::new(
                x,
                y,
                (rect.x + rect.w).min(clip.x + clip.w).max(x) - x,
                (rect.y + rect.h).min(clip.y + clip.h).max(y) - y,
            )
        } else {
            rect
        };
        if clip.w <= 0. || clip.h <= 0. {
            return;
        }
        self.stats.vector_triangles += mesh.vertices as usize / 3;
        self.batches.push(Batch {
            kind: BatchKind::Vector {
                path: path.into(),
                rect,
                tint,
            },
            clip: Some(clip),
            fade: self.fade,
        });
    }

    fn app_image(
        &mut self,
        image: &crate::app_artwork::Artwork,
        rect: Rect,
        radius: f32,
        tint: Color,
    ) {
        let entry = self.textures.entry(image.key.clone()).or_insert_with(|| {
            Some(texture(
                image.pixels.width(),
                image.pixels.height(),
                image.pixels.as_raw(),
            ))
        });
        let Some(texture) = entry else {
            return;
        };
        texture.frame = self.frame;
        let scale =
            (rect.w * 0.68 / texture.width as f32).min(rect.h * 0.68 / texture.height as f32);
        let w = texture.width as f32 * scale;
        let h = texture.height as f32 * scale;
        let id = texture.id;
        self.gradient_quad(
            rect,
            [
                0.5 - rect.w / (2. * w),
                0.5 - rect.h / (2. * h),
                0.5 + rect.w / (2. * w),
                0.5 + rect.h / (2. * h),
            ],
            radius,
            0.,
            [tint; 2],
            id,
            Some(image.backdrop),
        );
    }

    /// A logical damage rect in drawable pixels ([x, y, width, height], origin bottom-left),
    /// grown to whole pixels.
    pub fn damage_pixels(rect: Rect, width: u32, height: u32) -> [i32; 4] {
        let viewport = Viewport::new(Size::new(width, height));
        let sx = viewport.width as f32 / 1920.;
        let sy = viewport.height as f32 / 1080.;
        let x0 = (viewport.x as f32 + rect.x.max(0.) * sx).floor() as i32;
        let x1 = (viewport.x as f32 + (rect.x + rect.w).min(1920.) * sx).ceil() as i32;
        let y0 = (viewport.y as f32 + (1080. - (rect.y + rect.h).min(1080.)) * sy).floor() as i32;
        let y1 = (viewport.y as f32 + (1080. - rect.y.max(0.)) * sy).ceil() as i32;
        if x1 <= x0 || y1 <= y0 {
            // No change still names, clears and redraws one pixel inside the drawable. An empty
            // or off-surface region left the driver nothing to preserve: Mali presented black.
            return [viewport.x, viewport.y, 1, 1];
        }
        [x0, y0, x1 - x0, y1 - y0]
    }

    /// The logical area drawable pixels from `damage_pixels` cover. Grown to whole pixels it
    /// can exceed the original logical damage; commands are culled against this area, so none
    /// that paints into the cleared pixels is skipped.
    pub fn damage_area(pixels: [i32; 4], width: u32, height: u32) -> Rect {
        let viewport = Viewport::new(Size::new(width, height));
        let sx = viewport.width as f32 / 1920.;
        let sy = viewport.height as f32 / 1080.;
        let [x, y, w, h] = pixels;
        Rect::new(
            (x - viewport.x) as f32 / sx,
            1080. - (y + h - viewport.y) as f32 / sy,
            w as f32 / sx,
            h as f32 / sy,
        )
    }

    /// Prepares first-use work for a scene without drawing it (text layout, logo meshes,
    /// images, and glyphs requested from the worker without waiting), e.g. another page while
    /// the screen is idle, so its first appearance does not pay for it. The visible frame's
    /// statistics stay unchanged.
    pub fn prepare(&mut self, commands: &[Draw], width: u32, height: u32) {
        let stats = self.stats.clone();
        let awaited = std::mem::take(&mut self.awaited);
        self.build(commands, width, height, None, None);
        self.vertices.clear();
        self.glyph_vertices.clear();
        self.batches.clear();
        self.stats = stats;
        self.awaited = awaited;
    }

    /// Builds the frame. A full atlas is grown (once) or cleared and the frame built again
    /// from the glyphs' CPU copies; with `wait`, glyphs still being rasterised are waited for
    /// up to that long and the frame built again with them.
    fn build(
        &mut self,
        commands: &[Draw],
        width: u32,
        height: u32,
        area: Option<Rect>,
        wait: Option<std::time::Duration>,
    ) {
        let mut waited = 0.;
        let deadline = wait.map(|wait| Instant::now() + wait);
        for _ in 0..4 {
            self.build_once(commands, width, height, area);
            if self.atlas_full {
                if !(self.atlas.all_used_in(self.frame) && self.grow_atlas()) {
                    self.atlas.clear();
                }
                continue;
            }
            let Some(deadline) = deadline.filter(|_| !self.awaited.is_empty()) else {
                break;
            };
            let started = Instant::now();
            let mut arrived = false;
            while !self.awaited.is_empty() {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let results = self.glyph_worker.receive(Some(deadline - now));
                arrived |= self.accept(results);
            }
            waited += started.elapsed().as_secs_f64() * 1000.;
            if !arrived {
                break;
            }
        }
        self.stats.glyph_ms += waited;
    }

    fn grow_atlas(&mut self) -> bool {
        if !self.atlas.grow() {
            return false;
        }
        self.atlas_texture = atlas_texture(self.atlas.packer.size, self.gles);
        true
    }

    /// Lays out the scene into vertices and batches; commands entirely outside `area` are
    /// skipped.
    fn build_once(&mut self, commands: &[Draw], width: u32, height: u32, area: Option<Rect>) {
        self.stats = RenderStats::new(Size::new(width, height));
        self.atlas_full = false;
        self.awaited.clear();
        self.glyph_vertices.clear();
        // Captures can bind a single-sample FBO even when the actual TV window
        // is multisampled, so query the active target rather than assume.
        let mut samples = 0;
        unsafe { gl::GetIntegerv(gl::SAMPLES, &mut samples) };
        self.stats.multisample_samples = samples.max(0) as u32;
        self.frame += 1;
        self.vertices.clear();
        self.batches.clear();
        self.clip = None;
        self.fade = None;
        let mut index = 0;
        while index < commands.len() {
            let command = &commands[index];
            index += 1;
            if profile_skip(command) {
                continue;
            }
            if let Some(path) = plain_wallpaper(command)
                && self.clip.is_none()
                && self.fade.is_none()
            {
                let mut scrims = Vec::new();
                while let Some(next) = commands.get(index) {
                    if profile_skip(next) {
                        index += 1;
                    } else if let Some(scrim) = Scrim::of(next) {
                        scrims.push(scrim);
                        index += 1;
                    } else {
                        break;
                    }
                }
                if !scrims.is_empty() {
                    let screen = Rect::new(0., 0., 1920., 1080.);
                    self.folded_image(path, screen, 0., false, [1.; 4], &scrims);
                    continue;
                }
            }
            // Outside the damage the scissor would discard it anyway; skip its layout too.
            if let (Some(area), Some(bounds)) = (area, crate::damage::bounds(command))
                && !crate::damage::intersects(area, bounds)
            {
                continue;
            }
            match command {
                Draw::Shape(rect, radius, border, color) => self.quad(
                    *rect,
                    [0., 0., 1., 1.],
                    *radius,
                    *border,
                    *color,
                    self.white.id,
                ),
                Draw::RoundedGradient(rect, radius, top, bottom) => self.gradient_quad(
                    *rect,
                    [0., 0., 1., 1.],
                    *radius,
                    0.,
                    [*top, *bottom],
                    self.white.id,
                    None,
                ),
                Draw::Shadow(rect, radius, blur, color) => self.quad(
                    rect.inset(-*blur),
                    [0., 0., 1., 1.],
                    *radius,
                    -*blur,
                    *color,
                    self.white.id,
                ),
                Draw::Text(text, rect, size, color, centered) => {
                    self.text(text, *rect, *size, *color, *centered)
                }
                Draw::Image(path, rect, radius, contain) => {
                    self.image(path, *rect, *radius, *contain, [1.; 4])
                }
                Draw::TintedImage(path, rect, radius, contain, tint) => {
                    self.image(path, *rect, *radius, *contain, *tint)
                }
                Draw::AppImage(image, rect, radius, tint) => {
                    self.app_image(image, *rect, *radius, *tint)
                }
                Draw::Backdrop(path, rect, radius, tint) => {
                    self.backdrop(path, *rect, *radius, *tint)
                }
                Draw::Rim(rect, radius, width, top, bottom) => self.gradient_quad(
                    *rect,
                    [0., 0., 1., 1.],
                    *radius,
                    *width,
                    [*top, *bottom],
                    self.white.id,
                    None,
                ),
                Draw::Clip(clip) => self.clip = *clip,
                Draw::EdgeFade(fade) => self.fade = *fade,
            }
        }
    }

    /// Draws the scene. With `damage` (drawable pixels from `damage_pixels`), only those pixels
    /// are cleared and drawn; commands entirely outside them are skipped, and the rest of the
    /// back buffer keeps its earlier contents.
    pub fn render(&mut self, commands: &[Draw], width: u32, height: u32, damage: Option<[i32; 4]>) {
        // Until the first frame is shown, wait for all its glyphs: no text pops in at launch.
        let wait = if self.first_frame {
            GLYPH_WAIT_FIRST
        } else {
            GLYPH_WAIT
        };
        self.render_waiting(commands, width, height, damage, wait);
        self.first_frame = false;
    }

    fn render_waiting(
        &mut self,
        commands: &[Draw],
        width: u32,
        height: u32,
        damage: Option<[i32; 4]>,
        wait: std::time::Duration,
    ) {
        let area = damage.map(|pixels| Self::damage_area(pixels, width, height));
        let started = Instant::now();
        self.build(commands, width, height, area, Some(wait));
        let Viewport {
            x: vx,
            y: vy,
            width: vw,
            height: vh,
            scale,
        } = self.stats.viewport;
        if let Some([_, _, w, h]) = damage {
            self.stats.redrawn_fraction = f64::from(w * h) / f64::from((width * height).max(1));
        }
        let scissor = |area: Option<[i32; 4]>| unsafe {
            match area {
                Some([x, y, w, h]) => {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(x, y, w.max(0), h.max(0));
                }
                None => gl::Disable(gl::SCISSOR_TEST),
            }
        };
        unsafe {
            gl::Viewport(0, 0, width as i32, height as i32);
            scissor(damage);
            gl::ClearColor(0.04, 0.04, 0.05, 1.);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::Viewport(vx, vy, vw, vh);
            if self.vao != 0 {
                gl::BindVertexArray(self.vao);
            }
            gl::BindBuffer(gl::ARRAY_BUFFER, self.buffer);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (self.vertices.len() * size_of::<f32>()) as isize,
                self.vertices.as_ptr().cast(),
                gl::DYNAMIC_DRAW,
            );
            if !self.glyph_vertices.is_empty() {
                gl::BindBuffer(gl::ARRAY_BUFFER, self.glyph_buffer);
                gl::BufferData(
                    gl::ARRAY_BUFFER,
                    (self.glyph_vertices.len() * size_of::<f32>()) as isize,
                    self.glyph_vertices.as_ptr().cast(),
                    gl::DYNAMIC_DRAW,
                );
                gl::UseProgram(self.glyph_program);
                gl::Uniform1i(
                    gl::GetUniformLocation(self.glyph_program, c"u_atlas".as_ptr()),
                    0,
                );
                gl::Uniform1f(
                    gl::GetUniformLocation(self.glyph_program, c"u_light_exponent".as_ptr()),
                    LIGHT_TEXT_COVERAGE_EXPONENT,
                );
                gl::BindBuffer(gl::ARRAY_BUFFER, self.buffer);
            }
            gl::UseProgram(self.program);
            // One drawable pixel in logical units: the paint shader's edge filter width.
            gl::Uniform1f(
                gl::GetUniformLocation(self.program, c"u_aa".as_ptr()),
                1. / scale.max(0.01),
            );
            gl::Uniform1i(
                gl::GetUniformLocation(self.program, c"u_texture".as_ptr()),
                0,
            );
            gl::ActiveTexture(gl::TEXTURE0);
            let mut paint_bound = false;
            let mut current_fade = None;
            for batch in &self.batches {
                if current_fade != Some(batch.fade) {
                    // The same cheap alpha mask applies to shapes, textures,
                    // icons and text; the unmasked wallpaper remains beneath.
                    let fade = batch.fade.unwrap_or([0.; 4]);
                    for program in [self.program, self.vector_program, self.glyph_program] {
                        gl::UseProgram(program);
                        gl::Uniform4f(
                            gl::GetUniformLocation(program, c"u_edge_fade".as_ptr()),
                            fade[0],
                            fade[1],
                            fade[2],
                            fade[3],
                        );
                    }
                    current_fade = Some(batch.fade);
                    paint_bound = false;
                }
                let clip = batch.clip.map(|clip| {
                    [
                        vx + (clip.x * vw as f32 / 1920.).round() as i32,
                        vy + ((1080. - clip.y - clip.h) * vh as f32 / 1080.).round() as i32,
                        (clip.w * vw as f32 / 1920.).round() as i32,
                        (clip.h * vh as f32 / 1080.).round() as i32,
                    ]
                });
                scissor(match (clip, damage) {
                    (Some(a), Some(b)) => Some(intersect(a, b)),
                    (a, b) => a.or(b),
                });
                match &batch.kind {
                    BatchKind::Paint {
                        texture,
                        start,
                        count,
                        backdrop,
                    } => {
                        if !paint_bound {
                            gl::UseProgram(self.program);
                            gl::BindBuffer(gl::ARRAY_BUFFER, self.buffer);
                            let mut offset = 0;
                            for (index, size) in [2, 2, 4, 4, 2].into_iter().enumerate() {
                                gl::EnableVertexAttribArray(index as u32);
                                gl::VertexAttribPointer(
                                    index as u32,
                                    size,
                                    gl::FLOAT,
                                    gl::FALSE,
                                    56,
                                    (offset * size_of::<f32>()) as *const _,
                                );
                                offset += size as usize;
                            }
                            paint_bound = true;
                        }
                        gl::BindTexture(gl::TEXTURE_2D, *texture);
                        gl::Uniform1i(
                            gl::GetUniformLocation(self.program, c"u_app".as_ptr()),
                            i32::from(backdrop.is_some()),
                        );
                        if let Some(stops) = backdrop {
                            for (name, rgb) in
                                [(c"u_app_top", stops[0]), (c"u_app_bottom", stops[1])]
                            {
                                gl::Uniform3f(
                                    gl::GetUniformLocation(self.program, name.as_ptr()),
                                    rgb[0] as f32 / 255.,
                                    rgb[1] as f32 / 255.,
                                    rgb[2] as f32 / 255.,
                                );
                            }
                        }
                        gl::DrawArrays(gl::TRIANGLES, *start as i32, *count as i32);
                    }
                    BatchKind::Vector { path, rect, tint } => {
                        let Some(mesh) = self.vectors.get(path).and_then(Option::as_ref) else {
                            continue;
                        };
                        self.draw_mesh(
                            mesh,
                            [rect.w / mesh.width, rect.h / mesh.height, rect.x, rect.y],
                            *tint,
                        );
                        paint_bound = false;
                    }
                    BatchKind::Glyphs { start, count } => {
                        gl::UseProgram(self.glyph_program);
                        gl::BindBuffer(gl::ARRAY_BUFFER, self.glyph_buffer);
                        gl::DisableVertexAttribArray(3);
                        gl::DisableVertexAttribArray(4);
                        let mut offset = 0;
                        for (index, size) in [2, 2, 4].into_iter().enumerate() {
                            gl::EnableVertexAttribArray(index as u32);
                            gl::VertexAttribPointer(
                                index as u32,
                                size,
                                gl::FLOAT,
                                gl::FALSE,
                                32,
                                (offset * size_of::<f32>()) as *const _,
                            );
                            offset += size as usize;
                        }
                        gl::BindTexture(gl::TEXTURE_2D, self.atlas_texture.id);
                        gl::DrawArrays(gl::TRIANGLES, *start as i32, *count as i32);
                        paint_bound = false;
                    }
                }
            }
            gl::Disable(gl::SCISSOR_TEST);
        }
        // Keep the current scene, evict old full-resolution wallpapers before memory grows.
        let mut bytes: usize = self
            .textures
            .values()
            .flatten()
            .map(|t| t.width as usize * t.height as usize * 4)
            .sum();
        if bytes > TEXTURE_BUDGET {
            let mut old: Vec<_> = self
                .textures
                .iter()
                .filter_map(|(path, t)| {
                    t.as_ref()
                        .filter(|t| t.frame != self.frame)
                        .map(|t| (path.clone(), t.frame))
                })
                .collect();
            old.sort_by_key(|(_, frame)| *frame);
            for (path, _) in old {
                if bytes <= TEXTURE_BUDGET {
                    break;
                }
                if let Some(Some(t)) = self.textures.remove(&path) {
                    bytes -= t.width as usize * t.height as usize * 4;
                }
            }
        }
        self.trim_text();
        self.stats.vector_meshes = self.vectors.values().flatten().count();
        self.stats.vector_bytes = self.vectors.values().flatten().map(|mesh| mesh.bytes).sum();
        self.stats.artwork_bytes = bytes + self.stats.vector_bytes;
        self.stats.draw_calls = self.batches.len();
        self.stats.cpu_submission_ms = started.elapsed().as_secs_f64() * 1000.;
    }

    /// Draws a vector logo or icon: the exact outline under MSAA, else with its analytic
    /// one-pixel boundary ramp.
    fn draw_mesh(&self, mesh: &vector::Mesh, transform: [f32; 4], tint: Color) {
        let viewport = self.stats.viewport;
        let program = self.vector_program;
        unsafe {
            gl::UseProgram(program);
            gl::Uniform1f(
                gl::GetUniformLocation(program, c"u_fringe".as_ptr()),
                if self.stats.multisample_samples > 1 {
                    0.
                } else {
                    1.
                },
            );
            gl::BindBuffer(gl::ARRAY_BUFFER, mesh.buffer);
            gl::DisableVertexAttribArray(4);
            let mut offset = 0;
            for (index, size) in [2, 2, 4, 1].into_iter().enumerate() {
                gl::EnableVertexAttribArray(index as u32);
                gl::VertexAttribPointer(
                    index as u32,
                    size,
                    gl::FLOAT,
                    gl::FALSE,
                    36,
                    (offset * size_of::<f32>()) as *const _,
                );
                offset += size as usize;
            }
            gl::Uniform4f(
                gl::GetUniformLocation(program, c"u_transform".as_ptr()),
                transform[0],
                transform[1],
                transform[2],
                transform[3],
            );
            gl::Uniform2f(
                gl::GetUniformLocation(program, c"u_pixel_scale".as_ptr()),
                viewport.width as f32 / 1920.,
                viewport.height as f32 / 1080.,
            );
            gl::Uniform4f(
                gl::GetUniformLocation(program, c"u_tint".as_ptr()),
                tint[0],
                tint[1],
                tint[2],
                tint[3],
            );
            gl::DrawArrays(gl::TRIANGLES, 0, mesh.vertices);
        }
    }

    pub fn stats(&self) -> &RenderStats {
        &self.stats
    }

    pub fn screenshot(&self, path: &Path, width: u32, height: u32) -> Result<(), String> {
        let mut pixels = vec![0; width as usize * height as usize * 4];
        unsafe {
            gl::ReadPixels(
                0,
                0,
                width as i32,
                height as i32,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                pixels.as_mut_ptr().cast(),
            );
        }
        let mut image = image::RgbaImage::from_raw(width, height, pixels)
            .ok_or("Invalid framebuffer dimensions")?;
        image::imageops::flip_vertical_in_place(&mut image);
        image.save(path).map_err(|e| e.to_string())
    }

    /// Capture the authored TV surface, independent of desktop HiDPI/window limits.
    pub fn capture(
        &mut self,
        commands: &[Draw],
        path: &Path,
        size: Size,
    ) -> Result<RenderStats, String> {
        if size.width > self.info.max_texture_size
            || size.height > self.info.max_texture_size
            || size.width > self.info.max_viewport[0] as u32
            || size.height > self.info.max_viewport[1] as u32
        {
            return Err(format!(
                "Capture {}x{} exceeds GPU limits: {}px texture, {:?} viewport",
                size.width, size.height, self.info.max_texture_size, self.info.max_viewport
            ));
        }
        let image = texture(size.width, size.height, &[]);
        let mut framebuffer = 0;
        unsafe {
            gl::GenFramebuffers(1, &mut framebuffer);
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                image.id,
                0,
            );
            if gl::CheckFramebufferStatus(gl::FRAMEBUFFER) != gl::FRAMEBUFFER_COMPLETE {
                gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
                gl::DeleteFramebuffers(1, &framebuffer);
                return Err("Screenshot framebuffer is incomplete".into());
            }
        }
        // A capture waits for every glyph, as the first frame does.
        self.render_waiting(commands, size.width, size.height, None, GLYPH_WAIT_FIRST);
        let stats = self.stats.clone();
        let result = self.screenshot(path, size.width, size.height);
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            gl::DeleteFramebuffers(1, &framebuffer);
        }
        result.map(|_| stats)
    }

    /// Verification aid (LUMA_DAMAGE_VERIFY=1): compares the window's back buffer after a
    /// partial redraw with the same scene drawn in full offscreen at the window's sample count.
    /// Returns the pixels differing by more than 16 in a channel, and the largest difference.
    pub fn verify_window(
        &mut self,
        commands: &[Draw],
        width: u32,
        height: u32,
        save: Option<&Path>,
    ) -> (usize, u8) {
        let (w, h) = (width as i32, height as i32);
        let length = width as usize * height as usize * 4;
        let mut window = vec![0u8; length];
        let mut full = vec![0u8; length];
        let samples = self.stats.multisample_samples as i32;
        let stats = self.stats.clone();
        let mut framebuffers = [0; 2];
        let mut renderbuffers = [0; 2];
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            gl::ReadPixels(
                0,
                0,
                w,
                h,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                window.as_mut_ptr().cast(),
            );
            gl::GenFramebuffers(2, framebuffers.as_mut_ptr());
            gl::GenRenderbuffers(2, renderbuffers.as_mut_ptr());
            gl::BindRenderbuffer(gl::RENDERBUFFER, renderbuffers[0]);
            if samples > 1 {
                gl::RenderbufferStorageMultisample(gl::RENDERBUFFER, samples, gl::RGBA8, w, h);
            } else {
                gl::RenderbufferStorage(gl::RENDERBUFFER, gl::RGBA8, w, h);
            }
            gl::BindRenderbuffer(gl::RENDERBUFFER, renderbuffers[1]);
            gl::RenderbufferStorage(gl::RENDERBUFFER, gl::RGBA8, w, h);
            for (framebuffer, renderbuffer) in framebuffers.into_iter().zip(renderbuffers) {
                gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
                gl::FramebufferRenderbuffer(
                    gl::FRAMEBUFFER,
                    gl::COLOR_ATTACHMENT0,
                    gl::RENDERBUFFER,
                    renderbuffer,
                );
            }
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffers[0]);
        }
        self.render(commands, width, height, None);
        unsafe {
            gl::BindFramebuffer(gl::READ_FRAMEBUFFER, framebuffers[0]);
            gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, framebuffers[1]);
            gl::BlitFramebuffer(0, 0, w, h, 0, 0, w, h, gl::COLOR_BUFFER_BIT, gl::NEAREST);
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffers[1]);
            gl::ReadPixels(
                0,
                0,
                w,
                h,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                full.as_mut_ptr().cast(),
            );
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            gl::DeleteFramebuffers(2, framebuffers.as_ptr());
            gl::DeleteRenderbuffers(2, renderbuffers.as_ptr());
        }
        self.stats = stats;
        let mut differing = 0;
        let mut largest = 0;
        for (a, b) in window.chunks_exact(4).zip(full.chunks_exact(4)) {
            let difference = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0);
            largest = largest.max(difference);
            differing += usize::from(difference > 16);
        }
        if let Some(stem) = save.filter(|_| differing > 0) {
            for (name, pixels) in [("window", window), ("full", full)] {
                if let Some(image) = image::RgbaImage::from_raw(width, height, pixels) {
                    let image = image::imageops::flip_vertical(&image);
                    let _ = image.save(stem.with_extension(format!("{name}.png")));
                }
            }
        }
        (differing, largest)
    }
}

fn load_font(directory: &Path, id: u16) -> Result<Font, String> {
    let name = if id >= 1000 {
        format!("font-display-{}.ttf", id - 1000)
    } else {
        format!("font-{id}.ttf")
    };
    let path = directory.join(&name);
    let started = Instant::now();
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("Font {}: {e}. Regenerate native assets.", path.display()))?;
    let font = Font::from_bytes(bytes).map_err(|e| format!("Font {}: {e}", path.display()))?;
    if cfg!(feature = "webos") {
        eprintln!(
            "Luma font: {name} loaded in {} ms",
            started.elapsed().as_millis()
        );
    }
    Ok(font)
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.buffer);
            gl::DeleteBuffers(1, &self.glyph_buffer);
            gl::DeleteProgram(self.program);
            gl::DeleteProgram(self.vector_program);
            gl::DeleteProgram(self.glyph_program);
            if self.vao != 0 {
                gl::DeleteVertexArrays(1, &self.vao);
            }
        }
    }
}

fn atlas_format(gles: bool) -> GLenum {
    if gles { LUMINANCE } else { gl::RED }
}

/// The glyph atlas: one coverage channel, cleared, linearly filtered for scaled stand-ins.
fn atlas_texture(size: u32, gles: bool) -> Texture {
    let mut id = 0;
    let zeros = vec![0u8; size as usize * size as usize];
    unsafe {
        gl::GenTextures(1, &mut id);
        gl::BindTexture(gl::TEXTURE_2D, id);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            if gles { LUMINANCE } else { gl::R8 } as i32,
            size as i32,
            size as i32,
            0,
            atlas_format(gles),
            gl::UNSIGNED_BYTE,
            zeros.as_ptr().cast(),
        );
        for (name, value) in [
            (gl::TEXTURE_MIN_FILTER, gl::LINEAR),
            (gl::TEXTURE_MAG_FILTER, gl::LINEAR),
            (gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE),
            (gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE),
        ] {
            gl::TexParameteri(gl::TEXTURE_2D, name, value as i32);
        }
    }
    Texture {
        id,
        width: size,
        height: size,
        frame: 0,
    }
}

fn same_clip(a: Option<Rect>, b: Option<Rect>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h,
        (None, None) => true,
        _ => false,
    }
}

fn texture(width: u32, height: u32, pixels: &[u8]) -> Texture {
    let mut id = 0;
    unsafe {
        gl::GenTextures(1, &mut id);
        gl::BindTexture(gl::TEXTURE_2D, id);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as i32,
            width as i32,
            height as i32,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            if pixels.is_empty() {
                std::ptr::null()
            } else {
                pixels.as_ptr().cast()
            },
        );
    }
    Texture {
        id,
        width,
        height,
        frame: 0,
    }
}

fn gl_string(name: GLenum) -> String {
    unsafe {
        let value = gl::GetString(name);
        if value.is_null() {
            "unknown".into()
        } else {
            CStr::from_ptr(value.cast()).to_string_lossy().into_owned()
        }
    }
}

fn compile(kind: GLenum, source: &str) -> Result<GLuint, String> {
    let source = CString::new(source).map_err(|e| e.to_string())?;
    unsafe {
        let shader = gl::CreateShader(kind);
        gl::ShaderSource(shader, 1, &source.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);
        let mut success = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
        if success == 0 {
            let mut log = vec![0u8; 4096];
            let mut len = 0;
            gl::GetShaderInfoLog(shader, log.len() as i32, &mut len, log.as_mut_ptr().cast());
            gl::DeleteShader(shader);
            return Err(format!(
                "Shader compilation: {}",
                String::from_utf8_lossy(&log[..len as usize])
            ));
        }
        Ok(shader)
    }
}

fn link(vertex: &str, fragment: &str, attributes: &[&CStr]) -> Result<GLuint, String> {
    let vertex = compile(gl::VERTEX_SHADER, vertex)?;
    let fragment = match compile(gl::FRAGMENT_SHADER, fragment) {
        Ok(fragment) => fragment,
        Err(error) => {
            unsafe {
                gl::DeleteShader(vertex);
            }
            return Err(error);
        }
    };
    unsafe {
        let program = gl::CreateProgram();
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        for (index, name) in attributes.iter().enumerate() {
            gl::BindAttribLocation(program, index as u32, name.as_ptr());
        }
        gl::LinkProgram(program);
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
        let mut success = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);
        if success == 0 {
            let mut log = vec![0u8; 4096];
            let mut len = 0;
            gl::GetProgramInfoLog(program, log.len() as i32, &mut len, log.as_mut_ptr().cast());
            gl::DeleteProgram(program);
            return Err(format!(
                "Shader linking: {}",
                String::from_utf8_lossy(&log[..len as usize])
            ));
        }
        Ok(program)
    }
}
