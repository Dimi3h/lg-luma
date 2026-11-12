//! Screen geometry and styling mirror the web UI's 1920×1080 / 16px-rem layout.
use crate::{
    i18n::Language,
    model::{App, BACKGROUNDS, FocusGrow, Input, InputMode},
    renderer::{Color, Draw, TextStyle, WHITE},
    state::{
        APPS_COLUMNS, Control, KEYS, Overlay, Page, Rect, SYSTEM_SECTION, State, Subpage, Target,
        TextField, WIDGETS_SECTION,
    },
    weather::{self, Unit},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use time::OffsetDateTime;

/// Card artwork proportions are authored for this height; other sizes scale from it.
const FACE_HEIGHT: f32 = 154.4;
const RECENTS_BACKDROP_OPACITY: f32 = 0.5;

// tvOS layout (Human Interface Guidelines, tvOS points = logical pixels): an 80 × 60 safe
// area and, for six columns, 260-wide 5:3 cards with 40 between them and 100 between rows.
pub const SAFE_X: f32 = 80.;
pub const CARD_WIDTH: f32 = 260.;
pub const CARD_HEIGHT: f32 = 156.;
pub const CARD_PITCH: f32 = 300.;
/// Space between card rows: tighter than the HIG's 100 px; the focused caption (22 + 32 px)
/// still fits in it, as on tvOS.
pub const CARD_ROW_GAP: f32 = 76.;
pub const ROW_PITCH: f32 = CARD_HEIGHT + CARD_ROW_GAP;
/// The main screen's vertical rhythm: the clock's digits start this far below the top of the
/// screen, and the third card row ends this far above its bottom.
pub const SCREEN_MARGIN: f32 = 64.;
/// From the top of the clock's 36 px text box to the top of its digits (Inter at 29 px).
const CLOCK_CAP_INSET: f32 = 7.;
/// Rows of cards on the main screen before it scrolls.
pub const VISIBLE_ROWS: usize = 3;
/// The first card row: three rows end exactly `SCREEN_MARGIN` above the bottom.
pub const GRID_Y: f32 = 1080.
    - SCREEN_MARGIN
    - VISIBLE_ROWS as f32 * CARD_HEIGHT
    - (VISIBLE_ROWS - 1) as f32 * CARD_ROW_GAP;
const CARD_RADIUS: f32 = 18.;
/// Focus grows a card by this share around its centre, and rows and buttons by the second.
pub const ROW_LIFT: f32 = 0.025;
const BUTTON_LIFT: f32 = 0.06;
/// Rounded rectangles for lists and panels (tvOS uses rounded rectangles, not pills, here).
const ROW_RADIUS: f32 = 16.;
const PANEL_RADIUS: f32 = 30.;

// tvOS text styles (size / leading, Medium unless noted; Inter replaces SF Pro, and text of
// 32 px and up uses Inter Display). Tracking is light: titles about -2%, body text none.
// Nothing in Luma is smaller than Caption 2.
const TITLE2: TextStyle = TextStyle::new(57.)
    .weight(650)
    .tracking(-0.02)
    .leading(68. / 57.);
const TITLE3: TextStyle = TextStyle::new(48.)
    .weight(600)
    .tracking(-0.018)
    .leading(56. / 48.);
const HEADLINE: TextStyle = TextStyle::new(38.)
    .weight(600)
    .tracking(-0.015)
    .leading(46. / 38.);
const CALLOUT: TextStyle = TextStyle::new(31.)
    .weight(500)
    .tracking(-0.005)
    .leading(38. / 31.);
const BODY: TextStyle = TextStyle::new(29.).weight(500).leading(36. / 29.);
const CAPTION1: TextStyle = TextStyle::new(25.).weight(500).leading(32. / 25.);
const CAPTION2: TextStyle = TextStyle::new(23.).weight(500).leading(30. / 23.);

/// Text colours: primary white, and the secondary/tertiary greys tvOS uses on dark glass.
const SECONDARY: Color = [0.96, 0.96, 0.96, 0.66];
const TERTIARY: Color = [0.96, 0.96, 0.96, 0.46];
/// Dark label on a focused (white) surface.
const ON_LIGHT: Color = [0.08, 0.08, 0.09, 1.];
const ON_LIGHT_SECONDARY: Color = [0.08, 0.08, 0.09, 0.62];

#[path = "recents-view.rs"]
mod recents_view;

pub struct Assets {
    pub root: PathBuf,
    pub version: String,
    pub metrics: BTreeMap<String, BTreeMap<char, f32>>,
    pub artwork: crate::app_artwork::Service,
}
impl Assets {
    pub fn load(root: PathBuf, webos: bool) -> Result<Self, String> {
        let load = |file: &str| {
            std::fs::read(root.join(file))
                .map_err(|e| format!("Native assets are missing: {e}. Run pnpm assets."))
        };
        // Generated artwork may outlive a binary rebuild; version the running app, not the assets.
        let version = env!("CARGO_PKG_VERSION").to_owned();
        let metrics =
            serde_json::from_slice(&load("font-metrics.json")?).map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            version,
            metrics,
            artwork: crate::app_artwork::Service::new(webos),
        })
    }
    fn path(&self, path: impl AsRef<Path>) -> String {
        self.root.join(path).to_string_lossy().into_owned()
    }

    fn background(&self, state: &State, id: &str) -> String {
        if let Some(item) = state.wallpapers.iter().find(|item| item.id == id) {
            return item.image();
        }
        let id = if BACKGROUNDS.iter().any(|(known, _)| *known == id) {
            id
        } else {
            "dunes"
        };
        self.path(format!("wallpapers/{id}.jpg"))
    }

    /// The blurred copy of the visible background that glass panels sample.
    fn glass_backdrop(&self, state: &State, id: &str) -> Option<String> {
        let id = &id.to_owned();
        if let Some(item) = state.wallpapers.iter().find(|item| &item.id == id) {
            return item.blurred(true);
        }
        let id = if BACKGROUNDS.iter().any(|(known, _)| known == id) {
            id.as_str()
        } else {
            "dunes"
        };
        Some(self.path(format!("wallpapers/{id}-blur.jpg")))
    }

    /// One line no wider than `width`: shrinks to Caption 2 at most, then ends with "…".
    fn single_line(&self, value: &str, style: TextStyle, width: f32) -> (String, TextStyle) {
        let mut fitted = self.fit_text(value, style, width);
        if fitted.size >= CAPTION2.size.min(style.size) {
            return (value.to_owned(), fitted);
        }
        fitted.size = CAPTION2.size.min(style.size);
        let mut chars: Vec<char> = value.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
            if self.width(&candidate, fitted) <= width - 1. {
                return (candidate, fitted);
            }
        }
        ("…".into(), fitted)
    }

    fn width(&self, value: &str, style: TextStyle) -> f32 {
        let key = format!(
            "{}{}",
            if style.size >= 32. { "display-" } else { "" },
            style.weight
        );
        let advances = self.metrics.get(&key);
        value
            .chars()
            .map(|c| {
                (advances
                    .and_then(|a| a.get(&c))
                    .copied()
                    .unwrap_or(if c == ' ' { 0.28 } else { 0.53 })
                    + style.tracking)
                    * style.size
            })
            .sum()
    }

    fn fit_text(&self, value: &str, mut style: TextStyle, width: f32) -> TextStyle {
        // Leave a pixel for floating-point differences from the renderer's
        // font advances. Re-measure if shrinking crosses the display-font
        // threshold at 32px, where a different optical font is selected.
        let available = width - 1.;
        for _ in 0..2 {
            let measured = self.width(value, style);
            if measured <= available {
                break;
            }
            style.size *= available / measured;
        }
        style
    }

    fn lines(&self, value: &str, style: TextStyle, width: f32) -> usize {
        let key = format!(
            "{}{}",
            if style.size >= 32. { "display-" } else { "" },
            style.weight
        );
        let advances = self.metrics.get(&key);
        let advance = |c: char| {
            (advances
                .and_then(|a| a.get(&c))
                .copied()
                .unwrap_or(if c == ' ' { 0.28 } else { 0.53 })
                + style.tracking)
                * style.size
        };
        let mut lines = 1;
        let mut used = 0.;
        for word in value.split_inclusive(char::is_whitespace) {
            let w: f32 = word.chars().map(advance).sum();
            if used > 0. && used + w > width {
                lines += 1;
                used = 0.;
            }
            for c in word.chars() {
                if c == '\n' {
                    lines += 1;
                    used = 0.;
                    continue;
                }
                let w = advance(c);
                if used > 0. && used + w > width {
                    lines += 1;
                    used = 0.;
                }
                used += w;
            }
        }
        lines
    }
}

pub struct Scene {
    pub draw: Vec<Draw>,
    pub controls: Vec<Control>,
    pub modal: Option<Rect>,
    /// Blurred background for glass panels; `None` falls back to a translucent fill.
    backdrop: Option<String>,
    /// A focused list row, drawn after its neighbours so its soft shadow lies over them.
    raised: Vec<Draw>,
    /// Settings and overlays: focused controls appear at once in place (no lift), as cards do.
    static_focus: bool,
    /// How much larger focused app and wallpaper cards are (Settings › General › Focus size).
    focus: FocusGrow,
}

/// Grows `rect` around its centre by `share` of its size.
fn grow(rect: Rect, share: f32) -> Rect {
    let dx = rect.w * share / 2.;
    let dy = rect.h * share / 2.;
    Rect::new(rect.x - dx, rect.y - dy, rect.w + 2. * dx, rect.h + 2. * dy)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix_color(a: Color, b: Color, t: f32) -> Color {
    std::array::from_fn(|i| mix(a[i], b[i], t))
}

/// How a glass surface is filled. tvOS uses Liquid Glass for the functional layer only.
#[derive(Clone, Copy, PartialEq)]
enum Glass {
    /// Navigation, panels and controls over the wallpaper.
    Regular,
    /// Over app content rather than the wallpaper: a darker fill with the same rim.
    Thick,
}

impl Scene {
    fn new() -> Self {
        Self {
            draw: Vec::new(),
            controls: Vec::new(),
            modal: None,
            backdrop: None,
            raised: Vec::new(),
            static_focus: false,
            focus: FocusGrow::default(),
        }
    }
    /// How much a focused control grows: `lift`, or nothing where focus is static (Settings).
    fn lift(&self, lift: f32) -> f32 {
        if self.static_focus { 0. } else { lift }
    }
    fn shape(&mut self, rect: Rect, radius: f32, fill: Color) {
        self.draw.push(Draw::Shape(rect, radius, 0., fill));
    }
    fn border(&mut self, rect: Rect, radius: f32, width: f32, color: Color) {
        self.draw.push(Draw::Shape(rect, radius, width, color));
    }
    fn text(
        &mut self,
        text: impl Into<String>,
        rect: Rect,
        style: TextStyle,
        color: Color,
        center: bool,
    ) {
        self.draw
            .push(Draw::Text(text.into(), rect, style, color, center));
    }
    fn image(&mut self, path: String, rect: Rect, radius: f32, contain: bool) {
        self.draw.push(Draw::Image(path, rect, radius, contain));
    }
    fn icon(&mut self, assets: &Assets, name: &str, rect: Rect, color: Color) {
        self.draw.push(Draw::TintedImage(
            assets.path(format!("ui-icons/{name}.lvec")),
            rect,
            0.,
            true,
            color,
        ));
    }
    fn control(&mut self, target: Target, rect: Rect, scrollable: bool) {
        self.controls.push(Control {
            target,
            rect,
            scrollable,
        });
    }
    fn shadow(&mut self, rect: Rect, radius: f32, blur: f32, y: f32, alpha: f32) {
        self.draw.push(Draw::Shadow(
            Rect::new(rect.x, rect.y + y, rect.w, rect.h),
            radius,
            blur,
            c(0, alpha),
        ));
    }
    /// The tvOS focus shadow: small at rest, large and deep when lifted, so the rest of the
    /// screen appears farther away.
    /// A card's static shadow: the same at rest and focused, since cards no longer lift.
    fn resting_shadow(&mut self, rect: Rect, radius: f32) {
        self.lift_shadow(rect, radius, 0.);
    }
    fn lift_shadow(&mut self, rect: Rect, radius: f32, level: f32) {
        self.shadow(
            rect,
            radius,
            mix(14., 56., level),
            mix(5., 26., level),
            mix(0.16, 0.46, level),
        );
    }
    /// A Liquid Glass surface: the blurred, lightened background seen through it, a highlight
    /// along the top edge and a specular rim that is brightest at the top.
    fn glass(&mut self, rect: Rect, radius: f32, kind: Glass) {
        match (&self.backdrop, kind) {
            // The blurred copy already carries glass's light tint (see `tools/assets.mjs`),
            // so the panel costs one textured pass over its area.
            (Some(path), Glass::Regular) => {
                self.draw
                    .push(Draw::Backdrop(path.clone(), rect, radius, [1., 1., 1., 1.]));
            }
            _ => self.draw.push(Draw::RoundedGradient(
                rect,
                radius,
                c(0x3a414b, 0.8),
                c(0x262b33, 0.86),
            )),
        }
        // A soft highlight along the top edge, as tall as the corners so its rounding matches.
        let band = (2. * radius).min(rect.h).max(12.);
        self.draw.push(Draw::RoundedGradient(
            Rect::new(rect.x, rect.y, rect.w, band),
            radius.min(band / 2.),
            c(0xffffff, 0.08),
            c(0xffffff, 0.),
        ));
        // A thin specular edge: bright white along the top, a faint dark line at the bottom.
        self.draw.push(Draw::Rim(
            rect,
            radius,
            1.,
            c(0xffffff, 0.36),
            c(0x000000, 0.18),
        ));
    }
    /// A tvOS control in one of its focus states: glass at rest, an opaque white surface
    /// (with a light top-to-bottom shading) when focused or selected. Returns the drawn rect.
    fn button(&mut self, rect: Rect, radius: f32, level: f32, selected: bool, lift: f32) -> Rect {
        let drawn = grow(rect, self.lift(lift) * level);
        let radius = radius * drawn.h / rect.h;
        if level > 0. {
            self.lift_shadow(drawn, radius, level);
        }
        if selected && level < 1. {
            self.shadow(rect, radius, 10., 3., 0.18 * (1. - level));
            self.draw.push(Draw::RoundedGradient(
                rect,
                radius,
                c(0xffffff, 0.92),
                c(0xe9e9eb, 0.92),
            ));
        } else if level < 1. {
            self.glass(rect, radius, Glass::Regular);
        }
        if level > 0. {
            self.draw.push(Draw::RoundedGradient(
                drawn,
                radius,
                c(0xffffff, level),
                c(0xebebed, level),
            ));
        }
        drawn
    }
    /// Moves everything drawn from `from` by (dx, dy): the eased display scroll, while the
    /// controls keep their logical, unanimated positions.
    fn shift(&mut self, from: usize, dx: f32, dy: f32) {
        if dx == 0. && dy == 0. {
            return;
        }
        for draw in &mut self.draw[from..] {
            let rect = match draw {
                Draw::Shape(rect, ..)
                | Draw::RoundedGradient(rect, ..)
                | Draw::Shadow(rect, ..)
                | Draw::Text(_, rect, ..)
                | Draw::Image(_, rect, ..)
                | Draw::TintedImage(_, rect, ..)
                | Draw::AppImage(_, rect, ..)
                | Draw::Backdrop(_, rect, ..)
                | Draw::Rim(rect, ..) => rect,
                Draw::Clip(_) | Draw::EdgeFade(_) => continue,
            };
            rect.x += dx;
            rect.y += dy;
        }
    }
    fn fade(&mut self, from: usize, alpha: f32, brightness: f32) {
        for draw in &mut self.draw[from..] {
            let apply = |color: &mut Color| {
                for channel in &mut color[..3] {
                    *channel *= brightness;
                }
                color[3] *= alpha;
            };
            match draw {
                Draw::Shape(_, _, _, color)
                | Draw::Shadow(_, _, _, color)
                | Draw::Text(_, _, _, color, _)
                | Draw::TintedImage(_, _, _, _, color)
                | Draw::AppImage(_, _, _, color)
                | Draw::Backdrop(_, _, _, color) => apply(color),
                Draw::RoundedGradient(_, _, a, b) | Draw::Rim(_, _, _, a, b) => {
                    apply(a);
                    apply(b);
                }
                Draw::Image(path, rect, radius, contain) => {
                    *draw = Draw::TintedImage(
                        path.clone(),
                        *rect,
                        *radius,
                        *contain,
                        [brightness, brightness, brightness, alpha],
                    );
                }
                Draw::Clip(_) | Draw::EdgeFade(_) => {}
            }
        }
    }
}

/// The strings a companion dialog shows around its QR code, as message keys.
struct Companion {
    title: &'static str,
    keep_open: &'static str,
    private: &'static str,
    error: &'static str,
    starting: &'static str,
}

fn wallpaper_upload(out: &mut Scene, state: &State, assets: &Assets) {
    companion(
        out,
        state,
        assets,
        state.wallpaper_upload.as_ref(),
        Companion {
            title: "uploadWallpaper",
            keep_open: "wallpaperUploadKeepOpen",
            private: "wallpaperUploadPrivate",
            error: "wallpaperUploadError",
            starting: "wallpaperUploadStarting",
        },
    );
}

fn tile_editor(out: &mut Scene, state: &State, assets: &Assets) {
    companion(
        out,
        state,
        assets,
        state.tile_editor.as_ref(),
        Companion {
            title: "editTilesElsewhere",
            keep_open: "tileEditorKeepOpen",
            private: "tileEditorPrivate",
            error: "tileEditorError",
            starting: "tileEditorStarting",
        },
    );
}

/// A QR code and address for a page on the owner's phone or computer, with a Close button.
fn companion(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    session: Option<&Result<crate::companion::Session, String>>,
    strings: Companion,
) {
    let language = state.preferences.language;
    let panel = Rect::new(500., 96., 920., 888.);
    out.modal = Some(panel);
    out.shadow(panel, PANEL_RADIUS, 80., 24., 0.45);
    out.glass(panel, PANEL_RADIUS, Glass::Thick);
    out.text(
        language.t(strings.title),
        Rect::new(540., 134., 840., 56.),
        assets.fit_text(language.t(strings.title), TITLE3, 840.),
        WHITE,
        true,
    );
    out.text(
        language.t("wallpaperUploadHint"),
        Rect::new(560., 198., 800., 64.),
        CAPTION1.weight(400),
        SECONDARY,
        true,
    );
    match session {
        Some(Ok(session)) => {
            let scale = (336. / (session.qr_size + 8) as f32).floor();
            let size = (session.qr_size + 8) as f32 * scale;
            let x = (1920. - size) / 2.;
            let y = 284.;
            // Invert the normal QR polarity to blend into the dark modal, while
            // retaining a high-contrast foreground and the full quiet zone.
            let background = c(0x1b2023, 1.);
            let modules = c(0xf5f4ef, 1.);
            out.shape(Rect::new(x, y, size, size), 0., background);
            for row in 0..session.qr_size {
                // Merge contiguous modules so QR rendering stays a few hundred quads.
                let mut column = 0;
                while column < session.qr_size {
                    let mark_start = (session.qr_size - 11) / 2;
                    let in_mark = (mark_start..mark_start + 11).contains(&row)
                        && (mark_start..mark_start + 11).contains(&column);
                    if in_mark {
                        column += 1;
                        continue;
                    }
                    if !session.qr[(row * session.qr_size + column) as usize] {
                        column += 1;
                        continue;
                    }
                    let start = column;
                    while column < session.qr_size {
                        let in_mark = (mark_start..mark_start + 11).contains(&row)
                            && (mark_start..mark_start + 11).contains(&column);
                        if in_mark || !session.qr[(row * session.qr_size + column) as usize] {
                            break;
                        }
                        column += 1;
                    }
                    out.shape(
                        Rect::new(
                            x + (start + 4) as f32 * scale,
                            y + (row + 4) as f32 * scale,
                            (column - start) as f32 * scale,
                            scale,
                        ),
                        0.,
                        modules,
                    );
                }
            }
            let mark_start = (session.qr_size - 11) / 2;
            let mark_plate = Rect::new(
                x + (mark_start + 4) as f32 * scale,
                y + (mark_start + 4) as f32 * scale,
                11. * scale,
                11. * scale,
            );
            out.shape(mark_plate, 1.1 * scale, background);
            let mark = Rect::new(
                mark_plate.x + 1.25 * scale,
                mark_plate.y + 1.25 * scale,
                8.5 * scale,
                8.5 * scale,
            );
            out.icon(assets, "luma-mark", mark, WHITE);
            out.text(
                language.t("wallpaperUploadBrowser"),
                Rect::new(540., 636., 840., 32.),
                CAPTION1.weight(400),
                SECONDARY,
                true,
            );
            let display_url = session.url.trim_end_matches('/');
            out.text(
                display_url,
                Rect::new(540., 674., 840., 38.),
                assets.fit_text(display_url, CALLOUT.weight(600), 840.),
                WHITE,
                true,
            );
            out.text(
                language.t(strings.keep_open),
                Rect::new(540., 724., 840., 60.),
                CAPTION2.weight(400),
                SECONDARY,
                true,
            );
            out.text(
                language.t(strings.private),
                Rect::new(540., 790., 840., 30.),
                CAPTION2.weight(400),
                TERTIARY,
                true,
            );
        }
        result => {
            let key = if result.is_some() {
                strings.error
            } else {
                strings.starting
            };
            out.text(
                language.t(key),
                Rect::new(560., 460., 800., 108.),
                CALLOUT.weight(400),
                SECONDARY,
                true,
            );
        }
    }
    // The only action, so it is always the bright, prominent button.
    let close = Rect::new(800., 860., 320., 72.);
    let level = state.focus_level(&Target::CloseDialog);
    let drawn = out.button(close, 18., level, true, BUTTON_LIFT);
    out.text(
        language.t("close"),
        Rect::new(drawn.x, drawn.y + (drawn.h - 36.) / 2., drawn.w, 36.),
        BODY.weight(600),
        ON_LIGHT,
        true,
    );
    out.control(Target::CloseDialog, close, false);
}

fn c(rgb: u32, alpha: f32) -> Color {
    [
        ((rgb >> 16) & 255) as f32 / 255.,
        ((rgb >> 8) & 255) as f32 / 255.,
        (rgb & 255) as f32 / 255.,
        alpha,
    ]
}
fn text(size: f32, weight: u16, tracking: f32) -> TextStyle {
    TextStyle::new(size).weight(weight).tracking(tracking)
}

/// The clock line at the top of every page (tvOS has no tab bar on its Home Screen).
pub const HEADER: Rect = Rect::new(0., 46., 0., 68.);
/// Height of a one-line title row of controls (Settings' title line and its buttons).
pub const TITLE_LINE_H: f32 = 68.;

/// Fully shown Recents is drawn instead of the page, which is then not part of the scene.
pub fn recents_replaces_page(state: &State) -> bool {
    state.recents_open() && state.recents.visibility == 1.
}

pub fn scene(state: &State, assets: &Assets, now: OffsetDateTime) -> Scene {
    if let Some((seed, step)) = state.screensaver_position_step() {
        return screensaver(state, assets, now, seed, step);
    }
    let mut out = Scene::new();
    out.focus = state.preferences.focus_grow;
    out.static_focus =
        state.page == Page::Settings || state.overlay.is_some() || state.recents_open();
    let shown = state.preferences.background.clone();
    out.backdrop = assets.glass_backdrop(state, &shown);
    if let Some((previous, start)) = &state.previous_background {
        out.image(
            assets.background(state, previous),
            Rect::new(0., 0., 1920., 1080.),
            0.,
            false,
        );
        let alpha = (start.elapsed().as_secs_f32() / state.background_fade).min(1.);
        out.draw.push(Draw::TintedImage(
            assets.background(state, &shown),
            Rect::new(0., 0., 1920., 1080.),
            0.,
            false,
            [1., 1., 1., alpha],
        ));
    } else {
        // Recents covers the page with a dimmed, blurred stage: draw the blurred copy as the
        // background itself, so the blur costs no extra full-screen layer.
        let blurred = recents_replaces_page(state)
            .then(|| out.backdrop.clone())
            .flatten();
        out.image(
            blurred.unwrap_or_else(|| assets.background(state, &shown)),
            Rect::new(0., 0., 1920., 1080.),
            0.,
            false,
        );
    }
    let content_start = out.draw.len();
    if recents_replaces_page(state) {
        out.shape(
            Rect::new(0., 0., 1920., 1080.),
            0.,
            c(0x080b12, RECENTS_BACKDROP_OPACITY),
        );
        recents_view::render(&mut out, state, assets);
        if let Some((notice, _)) = &state.notice {
            toast(&mut out, state, assets, notice);
        }
        return out;
    }
    let viewport = state.content_viewport();
    let main_bottom = viewport.y + viewport.h;
    // No clip on the main screen: scrolled rows fade out beneath the clock instead.
    match state.page {
        Page::Home => home(&mut out, state, assets, now),
        Page::Settings => settings(&mut out, state, assets),
    }
    if let Some(error) = &state.error
        && !state.inventory.apps.is_empty()
        && !out
            .controls
            .iter()
            .any(|control| control.target == Target::Retry)
    {
        let rect = Rect::new(SAFE_X, main_bottom - 60. - 76., 1760., 76.);
        out.shadow(rect, 24., 30., 10., 0.3);
        out.glass(rect, 24., Glass::Regular);
        out.icon(
            assets,
            "alert",
            Rect::new(rect.x + 24., rect.y + 23., 30., 30.),
            c(0xf3ddbd, 1.),
        );
        let label = state.preferences.language.t("retry");
        let retry_width = assets.width(label, BODY.weight(600)) + 56.;
        let (error, style) = assets.single_line(error, CAPTION1, rect.w - 110. - retry_width);
        out.text(
            error,
            Rect::new(rect.x + 70., rect.y + 22., rect.w - 110. - retry_width, 32.),
            style,
            c(0xf3ddbd, 1.),
            false,
        );
        let retry = Rect::new(
            rect.x + rect.w - 10. - retry_width,
            rect.y + 10.,
            retry_width,
            56.,
        );
        segment(&mut out, state, Target::Retry, retry, label, false);
    }
    out.draw.push(Draw::EdgeFade(None));
    out.draw.push(Draw::Clip(None));
    header(&mut out, state, now);
    // The top bar stays put while the grid scrolls, like the clock.
    if state.page == Page::Home && state.overlay.is_none() && state.moving.is_none() {
        top_bar(&mut out, state, assets);
    }
    if let Some(
        overlay @ (Overlay::Options { .. }
        | Overlay::Delete { .. }
        | Overlay::BackgroundOptions { .. }
        | Overlay::BackgroundDelete { .. }
        | Overlay::Keyboard
        | Overlay::WallpaperUpload
        | Overlay::TileEditor),
    ) = &state.overlay
    {
        let anchor = match overlay {
            Overlay::BackgroundOptions { id, .. } => out.controls.iter()
                .find(|control| matches!(control.target, Target::Background(index) if state.background_id(index) == Some(id.as_str())))
                .map(|control| control.rect),
            // Card hit regions are exactly the artwork; captions appear only under focus.
            Overlay::Options { id, .. } => out
                .controls
                .iter()
                .find(|control| control.target == Target::App(id.clone()))
                .map(|control| control.rect),
            _ => None,
        };
        out.controls.clear();
        // tvOS dims and blurs the stage behind menus and alerts.
        if let Some(path) = out.backdrop.clone() {
            out.draw.push(Draw::Backdrop(
                path,
                Rect::new(0., 0., 1920., 1080.),
                0.,
                [1., 1., 1., 0.84],
            ));
            out.shape(Rect::new(0., 0., 1920., 1080.), 0., c(0x05070b, 0.38));
        } else {
            out.shape(Rect::new(0., 0., 1920., 1080.), 0., c(0x080b12, 0.72));
        }
        match overlay {
            Overlay::Options { id, .. } | Overlay::BackgroundOptions { id, .. } => {
                options(&mut out, state, assets, id, anchor)
            }
            Overlay::Delete { id, .. } | Overlay::BackgroundDelete { id, .. } => {
                delete_confirmation(&mut out, state, assets, id)
            }
            Overlay::Keyboard => keyboard(&mut out, state, assets),
            Overlay::WallpaperUpload => wallpaper_upload(&mut out, state, assets),
            Overlay::TileEditor => tile_editor(&mut out, state, assets),
            Overlay::SystemKeyboard | Overlay::Recents { .. } => {}
        }
    }
    if state.recents_open() || state.recents.visibility > 0. {
        // Crossfade the actual parent interface as well as the dimmed stage.
        // Retain outgoing visuals after dismissal, while input immediately
        // belongs to the restored parent. No geometry or hit targets move.
        let parent_controls = std::mem::take(&mut out.controls);
        let parent_modal = out.modal;
        out.fade(content_start, 1. - state.recents.visibility, 1.);
        let start = out.draw.len();
        out.shape(
            Rect::new(0., 0., 1920., 1080.),
            0.,
            c(0x080b12, RECENTS_BACKDROP_OPACITY),
        );
        recents_view::render(&mut out, state, assets);
        out.fade(start, state.recents.visibility, 1.);
        // Dim the wallpaper beneath the outgoing interface rather than over it. While both
        // fade, the backdrop and Recents' full-screen gradient then lie directly on the
        // wallpaper, and the renderer folds them into it instead of two more full-screen
        // passes per frame of the crossfade.
        let fills = out.draw[start..]
            .iter()
            .take_while(|draw| full_screen_fill(draw))
            .count();
        let mut moved: Vec<_> = out.draw.drain(start..start + fills).collect();
        if let Some(path) = out.backdrop.clone() {
            // Crossfade the blur in with the stage. Only during the transition this costs a
            // layer, and the fills above it then no longer fold into the wallpaper.
            moved.insert(
                0,
                Draw::TintedImage(
                    path,
                    Rect::new(0., 0., 1920., 1080.),
                    0.,
                    false,
                    [1., 1., 1., state.recents.visibility],
                ),
            );
        }
        out.draw.splice(content_start..content_start, moved);
        if !state.recents_open() {
            out.controls = parent_controls;
            out.modal = parent_modal;
        }
    }
    if let Some((notice, _)) = &state.notice {
        toast(&mut out, state, assets, notice);
    }
    if state.light(now) {
        light_appearance(&mut out, content_start);
    }
    out
}

/// The light appearance (Settings › General › Appearance): the finished scene, built for the
/// dark look, is brightened in one pass. A white scrim lightens the wallpaper, glass panels gain
/// a white layer, white text and interface icons turn near-black, dark veils turn white and
/// shadows soften. Focused white surfaces and their dark labels stay as they are.
fn light_appearance(out: &mut Scene, from: usize) {
    const INK: [f32; 3] = [0.09, 0.09, 0.1];
    let near_white = |color: &Color| color[0] > 0.8 && color[1] > 0.8 && color[2] > 0.8;
    let near_black = |color: &Color| color[0] < 0.25 && color[1] < 0.25 && color[2] < 0.25;
    let ink = |color: Color| [INK[0], INK[1], INK[2], color[3]];
    let mut draws = Vec::with_capacity(out.draw.len() + 8);
    for (index, mut draw) in std::mem::take(&mut out.draw).into_iter().enumerate() {
        if index == from {
            draws.push(Draw::Shape(
                Rect::new(0., 0., 1920., 1080.),
                0.,
                0.,
                [0.97, 0.97, 0.98, 0.5],
            ));
        }
        if index < from {
            draws.push(draw);
            continue;
        }
        let mut glass = None;
        match &mut draw {
            Draw::Text(_, _, _, color, _) if near_white(color) => *color = ink(*color),
            Draw::TintedImage(path, _, _, _, color)
                if path.contains("ui-icons/") && near_white(color) =>
            {
                *color = ink(*color)
            }
            Draw::Shape(_, _, 0., color) if near_black(color) => {
                *color = [0.96, 0.96, 0.97, color[3] * 0.9];
            }
            Draw::Shadow(_, _, _, color) => color[3] *= 0.5,
            Draw::Backdrop(_, rect, radius, color) => {
                glass = Some(Draw::Shape(
                    *rect,
                    *radius,
                    0.,
                    [1., 1., 1., 0.55 * color[3]],
                ));
            }
            _ => {}
        }
        draws.push(draw);
        draws.extend(glass);
    }
    if from >= draws.len() {
        draws.push(Draw::Shape(
            Rect::new(0., 0., 1920., 1080.),
            0.,
            0.,
            [0.97, 0.97, 0.98, 0.5],
        ));
    }
    out.draw = draws;
}

/// The clock's style and line; on the main screen the date follows it on the same line.
const CLOCK_STYLE: TextStyle = BODY.weight(550);
pub const CLOCK_Y: f32 = SCREEN_MARGIN - CLOCK_CAP_INSET;

fn header(out: &mut Scene, state: &State, now: OffsetDateTime) {
    let clock = state.preferences.language.clock(
        now,
        state.preferences.uses_24_hour(),
        state.preferences.show_seconds,
    );
    out.text(
        clock,
        Rect::new(SAFE_X, CLOCK_Y, 320., 36.),
        CLOCK_STYLE,
        c(0xffffff, 0.72),
        false,
    );
}

/// The screensaver's time: Title 1 scale in a light grey, and the date beneath, smaller and
/// dimmer, on pure black.
const SCREENSAVER_TIME: TextStyle = TextStyle::new(76.).weight(500).tracking(-0.02);
/// #bebebe and #7c7c7c.
const SCREENSAVER_TIME_COLOR: Color = [190. / 255., 190. / 255., 190. / 255., 1.];
const SCREENSAVER_DATE_COLOR: Color = [124. / 255., 124. / 255., 124. / 255., 1.];
const SCREENSAVER_TIME_H: f32 = 92.;
const SCREENSAVER_DATE_H: f32 = 40.;

/// The screensaver replaces the whole interface with a minimal digital clock on pure black
/// (the OLED's pixels off): the time and the date beneath it, in the clock and date formats
/// from Settings. The block jumps to a new random place inside the safe area every
/// `SCREENSAVER_STEP` against burn-in; nothing else is drawn and nothing animates in between.
fn screensaver(state: &State, assets: &Assets, now: OffsetDateTime, seed: u64, step: u64) -> Scene {
    let mut out = Scene::new();
    out.shape(Rect::new(0., 0., 1920., 1080.), 0., c(0x000000, 1.));
    let language = state.preferences.language;
    let time = language.clock(now, state.preferences.uses_24_hour(), false);
    let date = state.preferences.date_format.format_inline(language, now);
    let block = screensaver_block(assets, &time, &date, seed, step);
    // Around each jump the block fades out and in again; only its opacity changes.
    let alpha = state.screensaver_alpha();
    let faded = |color: Color| [color[0], color[1], color[2], color[3] * alpha];
    out.text(
        time,
        Rect::new(block.x, block.y, block.w, SCREENSAVER_TIME_H),
        SCREENSAVER_TIME,
        faded(SCREENSAVER_TIME_COLOR),
        false,
    );
    out.text(
        date,
        Rect::new(
            block.x,
            block.y + SCREENSAVER_TIME_H,
            block.w,
            SCREENSAVER_DATE_H,
        ),
        CALLOUT,
        faded(SCREENSAVER_DATE_COLOR),
        false,
    );
    out
}

/// The screensaver clock's block at position `step`: as wide as its longer line, placed at a
/// pseudo-random point (from `seed` and `step`) that keeps it whole inside the safe area.
pub fn screensaver_block(assets: &Assets, time: &str, date: &str, seed: u64, step: u64) -> Rect {
    let w = assets
        .width(time, SCREENSAVER_TIME)
        .max(assets.width(date, CALLOUT))
        .ceil()
        + 4.;
    let h = SCREENSAVER_TIME_H + SCREENSAVER_DATE_H;
    // SplitMix64: two well-mixed fractions per step.
    let mix = |mut z: u64| {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let a = mix(seed ^ step.wrapping_mul(0xd1b5_4a32_d192_ed03));
    let b = mix(a);
    let fraction = |value: u64| (value >> 11) as f32 / (1u64 << 53) as f32;
    let (left, top) = (SAFE_X, 60.);
    let (right, bottom) = (1920. - SAFE_X - w, 1080. - 60. - h);
    Rect::new(
        (left + fraction(a) * (right - left).max(0.)).round(),
        (top + fraction(b) * (bottom - top).max(0.)).round(),
        w,
        h,
    )
}

/// "Off" or "10 min".
fn screensaver_label(language: Language, minutes: u32) -> String {
    if minutes == 0 {
        language.t("stateOff").to_owned()
    } else {
        language.format("minutes", &[("count", &minutes.to_string())])
    }
}

/// Screensaver's choices: the owned labels need their own pass of the choice rows.
fn screensaver_choices(out: &mut Scene, state: &State, assets: &Assets, labels: Vec<String>) {
    let s = state.scroll;
    for (index, title) in labels.iter().enumerate() {
        row(
            out,
            state,
            assets,
            Row {
                target: Target::Choice(index),
                rect: settings_rect(SETTINGS_TOP + index as f32 * (ROW_H + ROW_GAP) - s, ROW_H),
                title,
                icon: None,
                value: None,
                description: None,
                selected: crate::state::SCREENSAVER_MINUTES[index]
                    == state.preferences.screensaver_minutes,
                chevron: false,
            },
        );
    }
}

/// An app card: at rest it sits flat with a small shadow; focus brightens it in place with a
/// light rim and shows its title beneath it. The hit region is the card.
fn tile(out: &mut Scene, state: &State, assets: &Assets, app: &App, rect: Rect, names: bool) {
    let target = Target::App(app.id.clone());
    out.control(target.clone(), rect, true);
    let viewport = state.content_viewport();
    // The eased display scroll can lag the logical position by up to a screen.
    let drift = (state.scroll - state.display_scroll()).abs();
    if rect.y + rect.h + CAPTION_SPACE < -drift
        || rect.y > 1054. + drift
        || rect.x + rect.w + 60. + drift < viewport.x
        || rect.x - 60. - drift > viewport.x + viewport.w
    {
        return;
    }
    let start = out.draw.len();
    let pressed = state.pressed.as_ref() == Some(&target);
    // Hidden apps appear only in search results, dimmed and without a shadow (unavailable).
    let unavailable = state.preferences.hidden.contains(&app.id);
    // Highlighted (pressed) returns to the unfocused size, tvOS's click feedback.
    let level = if pressed {
        0.
    } else {
        state.focus_level(&target)
    };
    // Nothing moves: focus changes instantly and the focused card only brightens. The card
    // being moved keeps that highlight with a slightly stronger rim; no jiggle, no arrows.
    let radius = CARD_RADIUS;
    // The size step follows focus alone, so pressing a card never changes its size or place.
    let drawn = if unavailable {
        rect
    } else {
        focus_card(out, rect, radius, state.focus_level(&target))
    };
    app_face(out, assets, app, drawn, state.preferences.language, radius);
    card_glass(out, drawn, radius, level);
    if state.moving.as_ref() == Some(&app.id) {
        out.draw.push(Draw::Rim(
            drawn,
            radius,
            MOVING_RIM,
            c(0xffffff, 0.9),
            c(0xffffff, 0.7),
        ));
    }
    if unavailable {
        out.fade(start, 0.45, 1.);
    }
    if pressed {
        out.fade(start, 1., 0.88);
    }
    if names && level > 0. {
        let title = caption_title(state, app);
        let (title, style) = assets.single_line(&title, BODY, rect.w + CARD_GAP_CAPTION);
        let area = caption_area(assets, &title, style, rect, 36.);
        out.text(title, area, style, c(0xffffff, level.min(1.)), true);
    }
    // The slot a dragged card would drop into is outlined and stays bright.
    let drop_target = state
        .drag
        .as_ref()
        .is_some_and(|drag| drag.active && drag.target.as_deref() == Some(app.id.as_str()));
    if state.moving.is_some() && state.moving.as_ref() != Some(&app.id) && !drop_target {
        out.fade(start, 0.48, 1.);
    }
    if drop_target {
        out.border(rect, CARD_RADIUS, 4., c(0xffffff, 0.95));
    }
}

/// Space below a card for its focused caption: the lift, a gap and one Body line.
pub const CAPTION_SPACE: f32 = 64.;
const CAPTION_GAP: f32 = 22.;
/// A caption may run a little wider than its card, as on tvOS.
const CARD_GAP_CAPTION: f32 = 36.;

/// The focused card's highlight over its face: about 6% brighter. No rim and no outline.
fn card_glass(out: &mut Scene, rect: Rect, radius: f32, level: f32) {
    if level > 0. {
        out.shape(rect, radius, c(0xffffff, FOCUS_OVERLAY * level.min(1.)));
    }
}

/// A grown card that would cross the screen margin is shifted back inside it, as long as the
/// card itself is inside (a card scrolled partly off screen keeps growing around its centre).
fn within_margin(card: Rect, grown: Rect) -> Rect {
    let shift = |low: f32, high: f32, card_low: f32, card_high: f32, from: f32, to: f32| {
        if card_low >= low - 0.5 && from < low {
            low - from
        } else if card_high <= high + 0.5 && to > high {
            high - to
        } else {
            0.
        }
    };
    let dx = shift(
        SCREEN_MARGIN,
        1920. - SCREEN_MARGIN,
        card.x,
        card.x + card.w,
        grown.x,
        grown.x + grown.w,
    );
    let dy = shift(
        SCREEN_MARGIN,
        1080. - SCREEN_MARGIN,
        card.y,
        card.y + card.h,
        grown.y,
        grown.y + grown.h,
    );
    Rect::new(grown.x + dx, grown.y + dy, grown.w, grown.h)
}

/// The focused card's drawn rectangle: its static size step, kept inside the screen margin.
fn focused_rect(focus: FocusGrow, rect: Rect) -> Rect {
    within_margin(rect, grow(rect, focus.share()))
}

/// A card's shadow and drawn rectangle for its focus `level`: the small resting shadow, or
/// the focus size step with a soft shadow that grows with it, so a larger step also looks more
/// lifted. The hit region is never changed; nothing animates.
fn focus_card(out: &mut Scene, rect: Rect, radius: f32, level: f32) -> Rect {
    if level <= 0. {
        out.resting_shadow(rect, radius);
        return rect;
    }
    let drawn = focused_rect(out.focus, rect);
    let (blur, y, alpha) = focus_shadow(out.focus);
    out.shadow(drawn, radius, blur, y, alpha);
    drawn
}

/// Blur, downward offset and opacity of the focused card's shadow: from soft and close at a
/// 6% step to wide, low and deeper at 14%. Light appearance halves shadow opacity.
pub fn focus_shadow(focus: FocusGrow) -> (f32, f32, f32) {
    let t = ((focus.share() - 0.06) / 0.08).clamp(0., 1.);
    (mix(32., 64., t), mix(12., 30., t), mix(0.32, 0.5, t))
}

/// How much brighter the focused card is: a white overlay of a few percent.
pub const FOCUS_OVERLAY: f32 = 0.06;

/// The card being moved: its rim, clearly visible in every focus style.
const MOVING_RIM: f32 = 3.5;

/// A caption's box: centred beneath its card, but kept inside the safe area so the first and
/// last cards' captions are not cut off at the screen edge.
fn caption_area(assets: &Assets, text: &str, style: TextStyle, card: Rect, height: f32) -> Rect {
    let width = assets.width(text, style).ceil() + 2.;
    let x = (card.x + (card.w - width) / 2.).clamp(SAFE_X - 12., 1920. - SAFE_X + 12. - width);
    Rect::new(x, card.y + card.h + CAPTION_GAP, width, height)
}

fn caption_title(state: &State, app: &App) -> String {
    if state.preferences.hidden.contains(&app.id) {
        format!(
            "{} · {}",
            app.title,
            state.preferences.language.t("hiddenLabel")
        )
    } else {
        app.title.clone()
    }
}

/// Shared card artwork for Home, Apps and Recents. Curated vectors always take
/// precedence over installed artwork, with the same relative logo proportions.
fn app_face(
    out: &mut Scene,
    assets: &Assets,
    app: &App,
    rect: Rect,
    language: Language,
    radius: f32,
) {
    let logo = crate::branding::for_key(&app.brand);
    let artwork = logo
        .is_none()
        .then(|| {
            app.icon
                .as_ref()
                .and_then(|source| assets.artwork.get(source))
        })
        .flatten();
    let installed = artwork.is_some();
    if let Some(image) = artwork {
        out.draw.push(Draw::AppImage(image, rect, radius, [1.; 4]));
    } else {
        out.image(
            assets.path(format!("surfaces/{}.png", app.brand)),
            rect,
            radius,
            false,
        );
    }
    if let Some(brand) = logo {
        let (ratio, height) = brand.artwork_size();
        let height = height * rect.h / FACE_HEIGHT;
        out.image(
            assets.path(format!("logos/{}.lvec", app.brand)),
            Rect::new(
                rect.x + rect.w * (1. - ratio) / 2.,
                rect.y + (rect.h - height) / 2.,
                rect.w * ratio,
                height,
            ),
            0.,
            true,
        );
    } else if !installed {
        brand_mark(out, assets, app, rect, language);
    }
}

fn brand_mark(out: &mut Scene, assets: &Assets, app: &App, rect: Rect, language: Language) {
    let scale = rect.h / FACE_HEIGHT;
    let rect = Rect::new(
        rect.x / scale,
        rect.y / scale,
        rect.w / scale,
        rect.h / scale,
    );
    let start = out.draw.len();
    let (cx, cy) = rect.center();
    match app.brand.as_str() {
        "plex" => {
            out.text(
                "ple",
                Rect::new(cx - 63., cy - 35.2, 130., 70.4),
                text(70.4, 650, -0.08).leading(1.),
                WHITE,
                false,
            );
            out.text(
                "x",
                Rect::new(cx + 20., cy - 35.2, 60., 70.4),
                text(70.4, 800, -0.08).leading(1.),
                c(0xe6aa25, 1.),
                false,
            );
        }
        "spotify" => {
            out.icon(
                assets,
                "spotify",
                Rect::new(cx - 77., cy - 20., 40., 40.),
                c(0x5ae08c, 1.),
            );
            out.text(
                "Spotify",
                Rect::new(cx - 29., cy - 14.4, 120., 36.),
                text(28.8, 650, -0.04).leading(1.),
                c(0x5ae08c, 1.),
                false,
            );
        }
        "kodi" => {
            out.icon(
                assets,
                "kodi",
                Rect::new(cx - 76., cy - 22.5, 45., 45.),
                WHITE,
            );
            out.text(
                "kodi",
                Rect::new(cx - 18., cy - 24.8, 130., 60.),
                text(49.6, 350, -0.04).leading(1.),
                WHITE,
                false,
            );
        }
        brand => {
            let (icon, label) = match brand {
                "live-tv" => ("tv", language.t("liveTV")),
                "homebrew" => ("coffee", language.t("homebrew")),
                "browser" => ("globe", language.t("browser")),
                "moonlight" => ("moonlight", "Moonlight"),
                "gaming" => ("gaming", app.title.as_str()),
                "custom" => (app.mark, app.title.as_str()),
                _ => ("generic", app.title.as_str()),
            };
            let font = if brand == "generic" {
                text(23., 500, -0.02).leading(1.25)
            } else {
                text(24., 500, -0.035).leading(1.2)
            };
            let w = (assets.width(label, font) + 54.4).min(rect.w - 32.);
            let label_w = rect.w - 54.4 - 32.;
            let label_h =
                (assets.lines(label, font, label_w) as f32 * font.size * font.line_height)
                    .min(rect.h - 32.);
            out.icon(
                assets,
                icon,
                Rect::new(cx - w / 2., cy - 20.8, 41.6, 41.6),
                WHITE,
            );
            out.text(
                label,
                Rect::new(cx - w / 2. + 54.4, cy - label_h / 2., label_w, label_h),
                font,
                WHITE,
                false,
            );
        }
    }
    if scale != 1. {
        for draw in &mut out.draw[start..] {
            let rect = match draw {
                Draw::Text(_, rect, style, _, _) => {
                    style.size *= scale;
                    rect
                }
                Draw::TintedImage(_, rect, _, _, _) => rect,
                _ => continue,
            };
            *rect = Rect::new(
                rect.x * scale,
                rect.y * scale,
                rect.w * scale,
                rect.h * scale,
            );
        }
    }
}

/// The top shelf fills what the three card rows leave: the date shares the clock's line, and
/// the greeting and one row of weather follow after a little whitespace.
pub const SHELF_TOP: f32 = CLOCK_Y + 36. + 28.;
pub const GREETING_HEIGHT: f32 = 68. + 6.;
pub const WEATHER_ROW_HEIGHT: f32 = 38.;
const GREETING_WIDTH: f32 = 1500.;
/// The shelf has faded out once the page has scrolled one row.
const SHELF_FADE: f32 = ROW_PITCH;

/// Logical top of the first card row (inputs when shown, then the apps). The grid is anchored
/// to the bottom margin, so hiding shelf widgets leaves it where it is.
pub fn grid_y(_state: &State) -> f32 {
    GRID_Y
}

/// Logical (unscrolled) top of the grid's first row.
pub fn grid_top(state: &State) -> f32 {
    grid_y(state)
}

/// Position of the `index`th card of a six-column block starting at `top`.
fn card_rect(index: usize, top: f32) -> Rect {
    Rect::new(
        SAFE_X + (index % APPS_COLUMNS) as f32 * CARD_PITCH,
        top + (index / APPS_COLUMNS) as f32 * ROW_PITCH,
        CARD_WIDTH,
        CARD_HEIGHT,
    )
}

/// The main screen, as the tvOS Home Screen: a top shelf (date, greeting, weather) above one
/// grid of every app, with Search first and Settings last. Moving below the first row scrolls
/// the shelf away (and dims it); returning to the first row brings it back.
fn home(out: &mut Scene, state: &State, assets: &Assets, now: OffsetDateTime) {
    let language = state.preferences.language;
    let s = state.scroll;
    // A constant dim over the wallpaper behind the shelf and grid, for legibility. It lies
    // directly on the wallpaper, so the renderer folds it into the wallpaper's own pass.
    if !state.light(now) {
        out.shape(Rect::new(0., 0., 1920., 1080.), 0., c(0x05070b, HOME_DIM));
    }
    let content_start = out.draw.len();
    let shelf_start = out.draw.len();
    if state.preferences.show_date {
        // On the clock's line, after the clock (drawn by `header`).
        let clock = language.clock(
            now,
            state.preferences.uses_24_hour(),
            state.preferences.show_seconds,
        );
        let x = SAFE_X + assets.width(&clock, CLOCK_STYLE).ceil() + 18.;
        let (date, style) = assets.single_line(
            &state.preferences.date_format.format_inline(language, now),
            CLOCK_STYLE.weight(450),
            1920. - SAFE_X - x,
        );
        out.text(
            date,
            Rect::new(x, CLOCK_Y - s, 1920. - SAFE_X - x, 36.),
            style,
            c(0xffffff, 0.56),
            false,
        );
    }
    // Scrolled rows fade out beneath the clock (the date above stays out of this fade).
    out.draw.push(Draw::EdgeFade(Some([
        HEADER.y + HEADER.h - 16.,
        HEADER.y + HEADER.h + 36.,
        1090.,
        1100.,
    ])));
    let top = SHELF_TOP - s;
    if state.preferences.show_greeting {
        let greeting = language.greeting(now.hour(), &state.preferences.greeting_name);
        let fitted = assets.fit_text(&greeting, TITLE2, GREETING_WIDTH);
        let line = TITLE2.size * TITLE2.line_height;
        let height = line * fitted.size / TITLE2.size;
        out.text(
            &greeting,
            Rect::new(
                SAFE_X - 3.,
                top + (line - height) / 2.,
                GREETING_WIDTH,
                height,
            ),
            fitted,
            WHITE,
            false,
        );
    }
    if state.preferences.show_weather {
        let mut summary = weather_summary_layout(state, assets);
        // One row only, so the shelf height (and the grid beneath it) never changes.
        summary.parts.retain(|part| part.3.y < WEATHER_ROW_HEIGHT);
        summary.height = summary.height.min(WEATHER_ROW_HEIGHT);
        let greeting_height = if state.preferences.show_greeting {
            GREETING_HEIGHT
        } else {
            0.
        };
        weather_summary(out, state, assets, summary, top + greeting_height);
    }
    // tvOS dims the shelf as focus leaves the first row; it follows the eased scroll.
    let shelf = (1. - state.display_scroll() / SHELF_FADE).clamp(0., 1.);
    if shelf < 1. {
        out.fade(shelf_start, shelf, 1.);
    }
    if state.moving.is_some() {
        // On the clock's line, at the right.
        out.text(
            language.t("moveApps"),
            Rect::new(1240., CLOCK_Y, 600., 36.),
            CAPTION1.right(),
            SECONDARY,
            false,
        );
    }
    let grid_y = grid_top(state) - s;
    let apps = state.apps();
    let names = state.preferences.show_app_names;
    let loading = state.loading && state.inventory.apps.is_empty();
    let error = state.inventory_failed_empty();
    if loading {
        for index in 0..APPS_COLUMNS {
            out.shape(card_rect(index, grid_y), CARD_RADIUS, c(0xffffff, 0.06));
        }
    }
    // A card dragged with the pointer is drawn last, over the others, where the pointer holds
    // it; its slot (and hit region) stays put until the drop.
    let dragged = state
        .drag
        .as_ref()
        .filter(|drag| drag.active)
        .map(|drag| drag.id.as_str());
    let mut lifted = None;
    for (index, app) in apps.iter().enumerate() {
        let rect = card_rect(index, grid_y);
        if dragged == Some(app.id.as_str()) {
            out.border(rect, CARD_RADIUS, 2., c(0xffffff, 0.35));
            lifted = Some((*app, rect));
            continue;
        }
        tile(out, state, assets, app, rect, names);
    }
    if let (Some((app, slot)), Some(drag)) = (lifted, &state.drag) {
        out.control(Target::App(app.id.clone()), slot, true);
        let held = Rect::new(
            drag.at.0 - drag.grab.0,
            drag.at.1 - drag.grab.1,
            slot.w,
            slot.h,
        );
        out.shadow(held, CARD_RADIUS, 40., 16., 0.45);
        app_face(
            out,
            assets,
            app,
            held,
            state.preferences.language,
            CARD_RADIUS,
        );
        card_glass(out, held, CARD_RADIUS, 1.);
    }
    // Without apps after a failed inventory, explain beneath the grid's cards.
    if error && !loading {
        let hint = state.error.clone().unwrap_or_default();
        empty(
            out,
            state,
            assets,
            EmptyState {
                rect: Rect::new(SAFE_X, grid_y + ROW_PITCH, 1760., 264.),
                icon: "alert",
                title: language.t("aLittleHelp"),
                hint: &hint,
                action: Some(Target::Retry),
            },
        );
    }
    out.draw.push(Draw::EdgeFade(None));
    out.shift(content_start, 0., s - state.display_scroll());
}

/// How much the main screen dims its wallpaper: constant, the same for every app and focus.
pub const HOME_DIM: f32 = 0.2;

/// The main screen's top bar, right-aligned on the clock's line: LG's app store (when
/// installed) and Settings as small glass circles with a glyph. Focus lifts
/// one onto a white circle and names it beneath.
pub const TOP_ICON: f32 = 44.;
const TOP_ICON_GAP: f32 = 14.;

/// Where the top bar's `index`th of `count` icons sits.
pub fn top_icon_rect(index: usize, count: usize) -> Rect {
    let x = 1920. - SAFE_X - TOP_ICON - (count - 1 - index) as f32 * (TOP_ICON + TOP_ICON_GAP);
    Rect::new(x, CLOCK_Y + 18. - TOP_ICON / 2., TOP_ICON, TOP_ICON)
}

/// Input glyphs in the top bar: this wide (focus area), left of the icons with a wider gap.
pub const TOP_INPUT: f32 = 40.;
const TOP_INPUT_GAP: f32 = 8.;
const TOP_GROUP_GAP: f32 = 30.;

/// Every top-bar item and its rectangle, left to right.
pub fn top_bar_rects(state: &State) -> Vec<(Target, Rect)> {
    let bar = state.top_bar();
    let icons = bar
        .iter()
        .filter(|target| !matches!(target, Target::Input(_)))
        .count();
    let inputs = bar.len() - icons;
    let first_icon = top_icon_rect(0, icons).x;
    bar.into_iter()
        .enumerate()
        .map(|(index, target)| {
            let rect = if index < inputs {
                let right = first_icon - TOP_GROUP_GAP;
                let x =
                    right - (inputs - index) as f32 * (TOP_INPUT + TOP_INPUT_GAP) + TOP_INPUT_GAP;
                Rect::new(x, CLOCK_Y + 18. - TOP_INPUT / 2., TOP_INPUT, TOP_INPUT)
            } else {
                top_icon_rect(index - inputs, icons)
            };
            (target, rect)
        })
        .collect()
}

/// A focused top-bar item's caption, beneath it and kept inside the safe area.
fn top_caption(out: &mut Scene, assets: &Assets, rect: Rect, text: &str, alpha: f32) {
    let (text, style) = assets.single_line(text, CAPTION2, 520.);
    let width = assets.width(&text, style).ceil() + 2.;
    let x = (rect.x + (rect.w - width) / 2.).min(1920. - SAFE_X - width);
    out.text(
        text,
        Rect::new(x, rect.y + rect.h + 10., width, 30.),
        style,
        c(0xffffff, 0.8 * alpha),
        false,
    );
}

/// An input in the top bar: its device glyph with a status dot, dim when nothing is connected.
/// Focus lifts it onto a small white circle and names it with its status beneath.
fn top_input(out: &mut Scene, state: &State, assets: &Assets, input: &Input, rect: Rect) {
    let target = Target::Input(input.id.clone());
    let level = state.display_level(&target);
    let drawn = grow(rect, out.lift(BUTTON_LIFT * 1.5) * level);
    if level > 0. {
        out.lift_shadow(drawn, drawn.h / 2., level);
        out.draw.push(Draw::RoundedGradient(
            drawn,
            drawn.h / 2.,
            c(0xffffff, level),
            c(0xebebed, level),
        ));
    }
    let connected = !matches!(input.connected, Some(false)) && input.signal != Some(false);
    let name = state.preferences.input_name(input);
    let icon = input_icon(input, name);
    let alpha = if connected { 0.86 } else { 0.38 };
    out.icon(
        assets,
        &format!("input-{icon}"),
        drawn.inset(6. * drawn.w / rect.w),
        mix_color(c(0xffffff, alpha), ON_LIGHT, level),
    );
    if connected {
        let dot = 7.;
        out.shape(
            Rect::new(drawn.x + drawn.w - dot - 1., drawn.y + 1., dot, dot),
            dot / 2.,
            c(0x5ad27c, 0.95),
        );
    }
    if level > 0. {
        let language = state.preferences.language;
        let caption = format!(
            "{} · {}",
            state.preferences.input_title(input),
            input_status(input, name.is_some(), language)
        );
        top_caption(out, assets, rect, &caption, level);
    }
    out.control(target, rect, false);
}

fn top_bar(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    for (target, rect) in top_bar_rects(state) {
        if let Target::Input(id) = &target {
            if let Some(input) = state.inventory.inputs.iter().find(|input| &input.id == id) {
                top_input(out, state, assets, input, rect);
            }
            continue;
        }
        let target = &target;
        let (icon, label) = match target {
            Target::StoreIcon => ("store", "storeApps"),
            _ => ("gear", "settings"),
        };
        let level = if state.pressed.as_ref() == Some(target) {
            0.
        } else {
            state.focus_level(target)
        };
        let drawn = grow(rect, out.lift(BUTTON_LIFT * 1.5) * level);
        if level > 0. {
            out.lift_shadow(drawn, drawn.h / 2., level);
        }
        if level < 1. {
            out.shape(rect, rect.h / 2., c(0xffffff, 0.12 * (1. - level)));
        }
        if level > 0. {
            out.draw.push(Draw::RoundedGradient(
                drawn,
                drawn.h / 2.,
                c(0xffffff, level),
                c(0xebebed, level),
            ));
            top_caption(out, assets, rect, language.t(label), level);
        }
        out.icon(
            assets,
            icon,
            drawn.inset(11. * drawn.w / rect.w),
            mix_color(c(0xffffff, 0.86), ON_LIGHT, level),
        );
        out.control(target.clone(), rect, false);
    }
}

struct WeatherSummary {
    height: f32,
    width: f32,
    icon: String,
    parts: Vec<(String, TextStyle, Color, Rect)>,
}
type WeatherPart = (String, TextStyle, Color, f32, f32);
const WEATHER_TEXT_X: f32 = SAFE_X + 46.;
const WEATHER_WIDTH: f32 = 1760. - 46.;
fn weather_summary_layout(state: &State, assets: &Assets) -> WeatherSummary {
    let language = state.preferences.language;
    let base = CALLOUT.weight(400);
    let mut parts = Vec::new();
    let mut icon = "location".to_owned();
    if let Some(place) = &state.preferences.weather_location {
        parts.push((place.name.clone(), base.weight(450), c(0xf5f5f5, 0.72)));
        parts.push(("·".into(), base, c(0xf5f5f5, 0.324)));
        if let Some(reading) = state.weather.reading(place) {
            let condition = weather::condition(reading);
            icon = format!("weather-{}", condition.icon);
            parts.push((
                weather::format_temperature(
                    reading.temperature,
                    state.preferences.temperature_unit,
                ),
                base.weight(550),
                c(0xf5f5f5, 0.91),
            ));
            parts.push((language.t(condition.key).into(), base, c(0xf5f5f5, 0.58)));
            if state.weather.failed {
                parts.push((
                    language.t("lastReading").into(),
                    CAPTION2,
                    c(0xf2d5a5, 0.65),
                ));
            }
        } else {
            parts.push((
                if state.weather.loading {
                    language.t("loadingWeather")
                } else {
                    language.t("weatherUnavailable")
                }
                .into(),
                base,
                c(0xf5f5f5, 0.58),
            ));
        }
    } else {
        parts.push((
            language.t("setWeatherLocation").into(),
            base,
            c(0xf5f5f5, 0.72),
        ));
    }
    let mut rows: Vec<Vec<WeatherPart>> = vec![Vec::new()];
    let mut used = 0.;
    for (label, style, color) in parts {
        let measured_width = assets.width(&label, style);
        let width = measured_width.min(WEATHER_WIDTH);
        // Use the same extra pixel as the rendered text rectangle. Counting
        // lines at the exact advance width can round into a phantom second
        // line even though the renderer displays the label on one line.
        let text_width = width + 1.;
        let lines = if measured_width <= text_width && !label.contains('\n') {
            1
        } else {
            assets.lines(&label, style, text_width)
        };
        let height = lines as f32 * style.size * style.line_height;
        if used > 0. && used + 12. + width > WEATHER_WIDTH {
            rows.push(Vec::new());
            used = 0.;
        }
        rows.last_mut()
            .unwrap()
            .push((label, style, color, width, height));
        used += width + if used > 0. { 12. } else { 0. };
    }
    let mut positioned = Vec::new();
    let mut top = 0.;
    let mut right: f32 = 0.;
    for row in rows {
        let height = row.iter().map(|part| part.4).fold(38., f32::max);
        let mut x = WEATHER_TEXT_X;
        for (label, style, color, width, item_height) in row {
            positioned.push((
                label,
                style,
                color,
                Rect::new(
                    x,
                    top + (height - item_height) / 2.,
                    width + 1.,
                    item_height,
                ),
            ));
            x += width + 12.;
        }
        right = right.max(x - 12.);
        top += height + 12.;
    }
    WeatherSummary {
        height: top - 12.,
        width: right - SAFE_X,
        icon,
        parts: positioned,
    }
}
fn weather_summary(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    summary: WeatherSummary,
    y: f32,
) {
    // Without a location the summary is a button: tvOS lifts it onto a white surface.
    let configurable = state.preferences.weather_location.is_none();
    let level = if configurable && !state.pointer {
        state.focus_level(&Target::WeatherConfigure)
    } else {
        0.
    };
    if level > 0. {
        let first = summary.parts.first().map_or(38., |part| part.3.h);
        let rect = grow(
            Rect::new(
                SAFE_X - 16.,
                y - 8.,
                summary.width + 32.,
                first.max(38.) + 16.,
            ),
            BUTTON_LIFT * level,
        );
        out.lift_shadow(rect, 16., level);
        out.draw.push(Draw::RoundedGradient(
            rect,
            16.,
            c(0xffffff, level),
            c(0xebebed, level),
        ));
    }
    out.icon(
        assets,
        &summary.icon,
        Rect::new(SAFE_X, y + 3., 32., 32.),
        mix_color(c(0xdedede, 0.85), ON_LIGHT, level),
    );
    for (label, style, color, rect) in summary.parts {
        out.text(
            label,
            Rect::new(rect.x, rect.y + y, rect.w, rect.h),
            style,
            mix_color(color, ON_LIGHT, level),
            false,
        );
    }
    if configurable {
        let rect = Rect::new(SAFE_X, y, summary.width, summary.height);
        out.control(Target::WeatherConfigure, rect, true);
    }
}

/// One segment of a tvOS tab bar: transparent at rest, white when selected, lifted with a
/// shadow when focused.
fn segment(
    out: &mut Scene,
    state: &State,
    target: Target,
    rect: Rect,
    label: &str,
    selected: bool,
) {
    let level = state.focus_level(&target);
    let pressed = state.pressed.as_ref() == Some(&target);
    let level = if pressed { 0. } else { level };
    let drawn = grow(rect, out.lift(BUTTON_LIFT) * level);
    if level > 0. {
        out.lift_shadow(drawn, drawn.h / 2., level);
    }
    let fill = if selected { 1. } else { level };
    if fill > 0. {
        out.draw.push(Draw::RoundedGradient(
            drawn,
            drawn.h / 2.,
            c(0xffffff, fill),
            c(0xebebed, fill),
        ));
    }
    out.text(
        label,
        Rect::new(drawn.x, drawn.y + (drawn.h - 36.) / 2., drawn.w, 36.),
        BODY.weight(if selected { 600 } else { 500 }),
        mix_color(c(0xffffff, 0.8), ON_LIGHT, fill),
        true,
    );
    if pressed {
        out.draw
            .push(Draw::Shape(drawn, drawn.h / 2., 0., c(0, 0.1)));
    }
    out.control(target, rect, false);
}

/// The existing System status card owns the only update action.
fn update_value(state: &State) -> String {
    use crate::update::{InstallStatus, Status};
    let language = state.preferences.language;
    match &state.update {
        InstallStatus::Idle => {
            if let Some(version) = &state.available_update {
                language.format("installUpdate", &[("version", version)])
            } else {
                language
                    .t(if state.update_status == Status::Checking {
                        "checkingUpdates"
                    } else {
                        "checkAgain"
                    })
                    .to_owned()
            }
        }
        InstallStatus::Downloading(percent) => {
            language.format("updateDownloading", &[("percent", &percent.to_string())])
        }
        InstallStatus::Installing(_) => language.t("updateInstalling").to_owned(),
        InstallStatus::Failed(_) => language.t("updateFailed").to_owned(),
    }
}

struct Row<'a> {
    target: Target,
    rect: Rect,
    title: &'a str,
    icon: Option<&'a str>,
    value: Option<&'a str>,
    description: Option<&'a str>,
    selected: bool,
    chevron: bool,
}

/// Moves draws made after a focused row (a value or a check drawn on top of it) up with it.
fn raise_with_row(out: &mut Scene, from: usize, level: f32) {
    if level > 0. {
        let extra = out.draw.split_off(from);
        out.raised.extend(extra);
    }
}

// Settings: a glass sidebar on the left, a title line and full-width rows on the right.
pub const SIDEBAR: Rect = Rect::new(SAFE_X, 144., 460., 432.);
pub const SIDEBAR_ITEMS_Y: f32 = SIDEBAR.y + 100.;
pub const SIDEBAR_PITCH: f32 = 80.;
pub const SETTINGS_X: f32 = 600.;
pub const SETTINGS_W: f32 = 1920. - SAFE_X - SETTINGS_X;
/// The content title line (section and breadcrumbs) and the first row beneath it.
pub const SETTINGS_TITLE_Y: f32 = 144.;
pub const SETTINGS_TOP: f32 = 236.;
pub const ROW_H: f32 = 84.;
pub const DETAIL_ROW_H: f32 = 112.;
pub const ROW_GAP: f32 = 12.;
const ROW_PAD: f32 = 28.;
const VALUE_WIDTH: f32 = 340.;

fn settings_rect(y: f32, h: f32) -> Rect {
    Rect::new(SETTINGS_X, y, SETTINGS_W, h)
}
/// The `index`th plain row of a settings list.
fn row_y(index: usize) -> f32 {
    SETTINGS_TOP + index as f32 * (ROW_H + ROW_GAP)
}

/// A tvOS list row: glass at rest; focused it turns white with dark text, grows slightly and
/// rounds into a lifted card. Values sit right-aligned in secondary grey.
fn row(out: &mut Scene, state: &State, assets: &Assets, row: Row<'_>) {
    let start = out.draw.len();
    let level = state.display_level(&row.target);
    let drawn = out.button(row.rect, ROW_RADIUS, level, false, ROW_LIFT);
    let ink = |rest: Color, focused: Color| mix_color(rest, focused, level);
    let primary = ink(WHITE, ON_LIGHT);
    let secondary = ink(SECONDARY, ON_LIGHT_SECONDARY);
    let mut x = drawn.x + ROW_PAD;
    if let Some(icon) = row.icon {
        out.icon(
            assets,
            icon,
            Rect::new(x, drawn.y + (drawn.h - 30.) / 2., 30., 30.),
            ink(c(0xffffff, 0.72), ON_LIGHT_SECONDARY),
        );
        x += 50.;
    }
    let title_y = drawn.y
        + if row.description.is_some() {
            20.
        } else {
            (drawn.h - 36.) / 2.
        };
    let right_inset = if row.value.is_some() {
        VALUE_WIDTH + 60.
    } else if matches!(row.target, Target::UploadWallpaper) {
        ROW_PAD
    } else {
        72.
    };
    let title_width = drawn.w - (x - drawn.x) - right_inset;
    let title_height = if row.description.is_some() {
        assets.lines(row.title, BODY, title_width) as f32 * 36.
    } else {
        36.
    };
    out.text(
        row.title,
        Rect::new(x, title_y, title_width, title_height),
        BODY,
        primary,
        false,
    );
    if let Some(description) = row.description {
        out.text(
            description,
            Rect::new(
                x,
                title_y + title_height + 4.,
                drawn.w - (x - drawn.x) - 80.,
                (drawn.h - 44. - title_height).max(32.),
            ),
            CAPTION1.weight(400),
            secondary,
            false,
        );
    }
    if let Some(value) = row.value {
        let value_width = VALUE_WIDTH;
        let (value, style) = assets.single_line(value, BODY.weight(400), value_width);
        out.text(
            value,
            Rect::new(
                drawn.x + drawn.w - value_width - ROW_PAD - if row.chevron { 34. } else { 0. },
                drawn.y + (drawn.h - 36.) / 2.,
                value_width,
                36.,
            ),
            style.right(),
            secondary,
            false,
        );
    }
    if row.chevron || row.selected {
        let size = if row.selected { 26. } else { 22. };
        out.icon(
            assets,
            if row.selected { "check" } else { "right" },
            Rect::new(
                drawn.x + drawn.w - ROW_PAD - size + 2.,
                drawn.y + (drawn.h - size) / 2.,
                size,
                size,
            ),
            if row.selected { primary } else { secondary },
        );
    }
    let scrollable = !matches!(row.target, Target::UploadWallpaper);
    if level > 0. {
        let lifted = out.draw.split_off(start);
        out.raised.extend(lifted);
    }
    out.control(row.target, row.rect, scrollable);
}

fn weather_settings(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let s = state.scroll;
    row(
        out,
        state,
        assets,
        Row {
            target: Target::WeatherToggle,
            rect: settings_rect(row_y(0) - s, ROW_H),
            title: language.t("showWidget"),
            icon: Some("eye"),
            value: Some(if state.preferences.show_weather {
                language.t("stateOn")
            } else {
                language.t("stateOff")
            }),
            description: None,
            selected: false,
            chevron: false,
        },
    );
    let city = state
        .preferences
        .weather_location
        .as_ref()
        .map(|place| place.name.as_str());
    let style = BODY.weight(400).leading(36. / 29.);
    let city_lines = city.map_or(1, |city| assets.lines(city, style, VALUE_WIDTH + 140.));
    let height = (48. + city_lines as f32 * 36.).max(ROW_H);
    let rect = settings_rect(row_y(1) - s, height);
    row(
        out,
        state,
        assets,
        Row {
            target: Target::WeatherLocation,
            rect,
            title: language.t("location"),
            icon: Some("location"),
            value: None,
            description: None,
            selected: false,
            chevron: true,
        },
    );
    if let Some(city) = city {
        let extra = out.draw.len();
        let level = state.focus_level(&Target::WeatherLocation);
        let drawn = grow(rect, out.lift(ROW_LIFT) * level);
        out.text(
            city,
            Rect::new(
                drawn.x + drawn.w - VALUE_WIDTH - 140. - ROW_PAD - 34.,
                drawn.y + (drawn.h - city_lines as f32 * 36.) / 2.,
                VALUE_WIDTH + 140.,
                city_lines as f32 * 36.,
            ),
            style.right(),
            mix_color(SECONDARY, ON_LIGHT_SECONDARY, level),
            false,
        );
        raise_with_row(out, extra, level);
    }
    let y = rect.y + height + ROW_GAP;
    row(
        out,
        state,
        assets,
        Row {
            target: Target::WeatherTemperature,
            rect: settings_rect(y, ROW_H),
            title: language.t("temperature"),
            icon: Some("thermometer"),
            value: Some(state.preferences.temperature_unit.symbol()),
            description: None,
            selected: false,
            chevron: true,
        },
    );
}

/// A glass text field that turns white when focused.
fn field(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    target: Target,
    rect: Rect,
    value: &str,
    placeholder: bool,
) {
    let level = state.focus_level(&target);
    let drawn = out.button(rect, rect.h / 2., level, false, ROW_LIFT);
    out.icon(
        assets,
        "search",
        Rect::new(drawn.x + 26., drawn.y + (drawn.h - 26.) / 2., 26., 26.),
        mix_color(c(0xffffff, 0.72), ON_LIGHT_SECONDARY, level),
    );
    out.text(
        value,
        Rect::new(
            drawn.x + 68.,
            drawn.y + (drawn.h - 36.) / 2.,
            drawn.w - 96.,
            36.,
        ),
        BODY.weight(400),
        mix_color(
            if placeholder { SECONDARY } else { WHITE },
            if placeholder {
                ON_LIGHT_SECONDARY
            } else {
                ON_LIGHT
            },
            level,
        ),
        false,
    );
    out.control(target, rect, true);
}

/// A round glass icon button with the tvOS lift.
fn round_button(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    target: Target,
    rect: Rect,
    icon: &str,
) {
    let level = state.focus_level(&target);
    let drawn = out.button(rect, rect.h / 2., level, false, BUTTON_LIFT);
    let size = rect.w * 0.4 * drawn.w / rect.w;
    out.icon(
        assets,
        icon,
        Rect::new(
            drawn.x + (drawn.w - size) / 2.,
            drawn.y + (drawn.h - size) / 2.,
            size,
            size,
        ),
        mix_color(WHITE, ON_LIGHT, level),
    );
    out.control(target, rect, true);
}

fn location_settings(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let mut y = SETTINGS_TOP - state.scroll;
    if let Some(place) = &state.preferences.weather_location {
        let rect = settings_rect(y, DETAIL_ROW_H);
        out.glass(rect, ROW_RADIUS, Glass::Regular);
        out.icon(
            assets,
            "location",
            Rect::new(rect.x + ROW_PAD, rect.y + (rect.h - 30.) / 2., 30., 30.),
            c(0xffffff, 0.72),
        );
        out.text(
            &place.name,
            Rect::new(rect.x + ROW_PAD + 50., y + 20., rect.w - 220., 36.),
            BODY,
            WHITE,
            false,
        );
        out.text(
            place.detail(),
            Rect::new(rect.x + ROW_PAD + 50., y + 60., rect.w - 220., 32.),
            CAPTION1.weight(400),
            SECONDARY,
            false,
        );
        round_button(
            out,
            state,
            assets,
            Target::WeatherClear,
            Rect::new(
                rect.x + rect.w - 24. - 64.,
                y + (rect.h - 64.) / 2.,
                64.,
                64.,
            ),
            "close",
        );
        y += DETAIL_ROW_H + 24.;
    }
    let query = Rect::new(SETTINGS_X, y, SETTINGS_W - ROW_H - 16., ROW_H);
    let empty = state.weather.query.is_empty();
    field(
        out,
        state,
        assets,
        Target::WeatherQuery,
        query,
        if empty {
            language.t("cityPostcode")
        } else {
            &state.weather.query
        },
        empty,
    );
    round_button(
        out,
        state,
        assets,
        Target::WeatherSearch,
        Rect::new(SETTINGS_X + SETTINGS_W - ROW_H, y, ROW_H, ROW_H),
        if state.weather.searching {
            "refresh"
        } else {
            "search"
        },
    );
    y += ROW_H + 24.;
    if state.weather.searching || state.weather.message.is_some() {
        out.text(
            if state.weather.searching {
                language.t("searching")
            } else {
                language.t(state.weather.message.as_deref().unwrap_or(""))
            },
            Rect::new(SETTINGS_X + ROW_PAD, y, SETTINGS_W - 2. * ROW_PAD, 64.),
            CAPTION1.weight(400),
            SECONDARY,
            false,
        );
    }
    for (index, place) in state.weather.results.iter().enumerate() {
        let detail = place.detail();
        let text_width = SETTINGS_W - 2. * ROW_PAD - 50. - 72.;
        let height = (44.
            + assets.lines(&place.name, BODY, text_width) as f32 * 36.
            + 4.
            + assets.lines(&detail, CAPTION1.weight(400), text_width) as f32 * 32.)
            .max(DETAIL_ROW_H);
        let rect = settings_rect(y, height);
        let target = Target::WeatherResult(index);
        row(
            out,
            state,
            assets,
            Row {
                target: target.clone(),
                rect,
                title: &place.name,
                icon: Some("location"),
                value: None,
                description: Some(&detail),
                selected: false,
                chevron: false,
            },
        );
        if state
            .preferences
            .weather_location
            .as_ref()
            .is_some_and(|saved| saved.key() == place.key())
        {
            let extra = out.draw.len();
            let level = state.focus_level(&target);
            let drawn = grow(rect, out.lift(ROW_LIFT) * level);
            out.icon(
                assets,
                "check",
                Rect::new(
                    drawn.x + drawn.w - ROW_PAD - 24.,
                    drawn.y + (drawn.h - 26.) / 2.,
                    26.,
                    26.,
                ),
                mix_color(WHITE, ON_LIGHT, level),
            );
            raise_with_row(out, extra, level);
        }
        y += height + ROW_GAP;
    }
}

fn settings(out: &mut Scene, state: &State, assets: &Assets) {
    // Lists scroll beneath the title line and fade out just above the first row, and fade out
    // again towards the bottom safe-area line, so no row is cut at the screen edge.
    out.draw.push(Draw::EdgeFade(Some([
        SETTINGS_TOP - 36.,
        SETTINGS_TOP - 10.,
        crate::state::SETTINGS_SAFE_BOTTOM - crate::state::SETTINGS_LIST_FADE,
        crate::state::SETTINGS_SAFE_BOTTOM,
    ])));
    let start = out.draw.len();
    settings_content(out, state, assets);
    let raised = std::mem::take(&mut out.raised);
    out.draw.extend(raised);
    out.shift(start, 0., state.scroll - state.display_scroll());
    out.draw.push(Draw::EdgeFade(None));
    settings_title(out, state, assets);
    if state.section == 1 && state.subpage.is_none() {
        background_actions(out, state, assets);
        let raised = std::mem::take(&mut out.raised);
        out.draw.extend(raised);
    }
    settings_sidebar(out, state, assets);
}

/// The section and the subpage path, as a title line above the content column (tvOS shows
/// where you are as a title rather than inside the selected sidebar item).
fn settings_path(state: &State) -> Vec<&str> {
    let language = state.preferences.language;
    let section = language.t(["interface", "background", "widgets", "system"][state.section]);
    let nested = match &state.subpage {
        Some(Subpage::DateFormat) => vec![language.t("date"), language.t("dateFormat")],
        Some(Subpage::ClockFormat) => vec![language.t("clock"), language.t("clockFormat")],
        Some(Subpage::TimeFormat) => vec![language.t("clock"), language.t("timeFormat")],
        Some(Subpage::WeatherLocation) => vec![language.t("weather"), language.t("location")],
        Some(Subpage::Temperature) => vec![language.t("weather"), language.t("temperature")],
        Some(Subpage::Input(id)) => vec![
            language.t("inputs"),
            state
                .inventory
                .inputs
                .iter()
                .find(|input| &input.id == id)
                .map_or(language.t("inputs"), |input| {
                    state.preferences.input_title(input)
                }),
        ],
        Some(Subpage::Greeting) => vec![language.t("greeting")],
        Some(Subpage::Date) => vec![language.t("date")],
        Some(Subpage::Weather) => vec![language.t("weather")],
        Some(Subpage::Shortcuts) => vec![language.t("shortcuts")],
        Some(Subpage::Inputs) => vec![language.t("inputs")],
        Some(Subpage::Clock) => vec![language.t("clock")],
        Some(Subpage::Language) => vec![language.t("language")],
        Some(Subpage::HiddenApps) => vec![language.t("hiddenApps")],
        Some(Subpage::Screensaver) => vec![language.t("screensaver")],
        Some(Subpage::Appearance) => vec![language.t("appearance")],
        None => vec![],
    };
    let mut path = vec![section];
    path.extend(nested);
    path
}

pub const BREADCRUMB_GAP: f32 = 12.;
const BREADCRUMB_CHEVRON: f32 = 28.;

fn settings_title(out: &mut Scene, state: &State, assets: &Assets) {
    let path = settings_path(state);
    // The Background actions share this line on the right.
    let available = if state.section == 1 && state.subpage.is_none() {
        SETTINGS_W * 0.36
    } else {
        SETTINGS_W
    };
    let separators = (path.len() - 1) as f32 * (BREADCRUMB_GAP * 2. + BREADCRUMB_CHEVRON);
    let mut style = HEADLINE;
    let text_width: f32 = path.iter().map(|part| assets.width(part, style)).sum();
    style.size *= ((available - separators) / text_width).min(1.);
    let y = SETTINGS_TITLE_Y + (TITLE_LINE_H - 46.) / 2.;
    let mut x = SETTINGS_X + 4.;
    for (position, &part) in path.iter().enumerate() {
        if position > 0 {
            out.icon(
                assets,
                "right",
                Rect::new(
                    x + BREADCRUMB_GAP,
                    y + (46. - BREADCRUMB_CHEVRON) / 2.,
                    BREADCRUMB_CHEVRON,
                    BREADCRUMB_CHEVRON,
                ),
                TERTIARY,
            );
            x += BREADCRUMB_GAP * 2. + BREADCRUMB_CHEVRON;
        }
        let width = assets.width(part, style);
        out.text(
            part,
            Rect::new(x, y, width + 2., 46.),
            style,
            if position == path.len() - 1 {
                WHITE
            } else {
                SECONDARY
            },
            false,
        );
        x += width;
    }
}

fn settings_sidebar(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    out.shadow(SIDEBAR, PANEL_RADIUS, 40., 14., 0.22);
    out.glass(SIDEBAR, PANEL_RADIUS, Glass::Regular);
    out.text(
        language.t("settings"),
        Rect::new(SIDEBAR.x + 28., SIDEBAR.y + 24., SIDEBAR.w - 56., 56.),
        assets.fit_text(language.t("settings"), TITLE3, SIDEBAR.w - 56.),
        WHITE,
        false,
    );
    for (index, key) in ["interface", "background", "widgets", "system"]
        .into_iter()
        .enumerate()
    {
        let label = language.t(key);
        let rect = Rect::new(
            SIDEBAR.x + 16.,
            SIDEBAR_ITEMS_Y + index as f32 * SIDEBAR_PITCH,
            SIDEBAR.w - 32.,
            SIDEBAR_PITCH - 8.,
        );
        let target = Target::Sidebar(index);
        // The selected section stays highlighted (translucent) while focus is elsewhere.
        let selected = state.section == index;
        let level = state.display_level(&target);
        let drawn = grow(rect, out.lift(BUTTON_LIFT / 2.) * level);
        if level > 0. {
            out.lift_shadow(drawn, ROW_RADIUS, level);
        }
        if selected && level < 1. {
            out.shape(rect, ROW_RADIUS, c(0xffffff, 0.16 * (1. - level)));
        }
        if level > 0. {
            out.draw.push(Draw::RoundedGradient(
                drawn,
                ROW_RADIUS,
                c(0xffffff, level),
                c(0xebebed, level),
            ));
        }
        let rest = if selected { WHITE } else { c(0xffffff, 0.74) };
        let (label, style) = assets.single_line(
            label,
            BODY.weight(if selected { 600 } else { 500 }),
            rect.w - 2. * 22. - 30.,
        );
        out.text(
            label,
            Rect::new(
                drawn.x + 22.,
                drawn.y + (drawn.h - 36.) / 2.,
                rect.w - 74.,
                36.,
            ),
            style,
            mix_color(rest, ON_LIGHT, level),
            false,
        );
        if selected {
            out.icon(
                assets,
                "right",
                Rect::new(
                    drawn.x + drawn.w - 22. - 22.,
                    drawn.y + (drawn.h - 22.) / 2.,
                    22.,
                    22.,
                ),
                mix_color(c(0xffffff, 0.5), ON_LIGHT_SECONDARY, level),
            );
        }
        out.control(target, rect, false);
    }
}

/// Settings › General › Hidden apps: one row per hidden app; OK shows it on the main screen
/// again. Without hidden apps an empty state explains where they come from.
fn hidden_apps_settings(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let s = state.scroll;
    let apps = state.hidden_apps();
    if apps.is_empty() {
        let rect = settings_rect(SETTINGS_TOP - s, 240.);
        empty(
            out,
            state,
            assets,
            EmptyState {
                rect,
                icon: "eye-off",
                title: language.t("noHidden"),
                hint: language.t("hiddenHint"),
                action: None,
            },
        );
        // The empty state holds focus, so Back and Left behave as on any subpage.
        out.control(Target::Choice(0), rect, false);
        return;
    }
    for (index, app) in apps.iter().enumerate() {
        row(
            out,
            state,
            assets,
            Row {
                target: Target::Choice(index),
                rect: settings_rect(SETTINGS_TOP + index as f32 * (ROW_H + ROW_GAP) - s, ROW_H),
                title: &app.title,
                icon: None,
                value: Some(language.t("showApp")),
                description: None,
                selected: false,
                chevron: false,
            },
        );
    }
}

fn settings_content(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let s = state.scroll;
    if let Some(subpage) = &state.subpage {
        if matches!(
            subpage,
            Subpage::WeatherLocation
                | Subpage::Weather
                | Subpage::Shortcuts
                | Subpage::Inputs
                | Subpage::Greeting
                | Subpage::Date
                | Subpage::Clock
                | Subpage::HiddenApps
        ) {
            match subpage {
                Subpage::HiddenApps => hidden_apps_settings(out, state, assets),
                Subpage::WeatherLocation => location_settings(out, state, assets),
                Subpage::Weather => weather_settings(out, state, assets),
                Subpage::Shortcuts => shortcut_settings(out, state, assets),
                Subpage::Inputs => inputs_settings(out, state, assets),
                Subpage::Greeting => greeting_settings(out, state, assets),
                Subpage::Date => date_settings(out, state, assets),
                Subpage::Clock => clock_settings(out, state, assets),
                _ => unreachable!(),
            }
            return;
        }
        let options: Vec<(&str, Option<&str>, bool)> = match subpage {
            Subpage::DateFormat => crate::model::DateFormat::ALL
                .iter()
                .map(|format| {
                    (
                        format.label(language),
                        None,
                        *format == state.preferences.date_format,
                    )
                })
                .collect(),
            Subpage::ClockFormat => vec![
                (
                    language.t("hour12"),
                    None,
                    !state.preferences.uses_24_hour(),
                ),
                (language.t("hour24"), None, state.preferences.uses_24_hour()),
            ],
            Subpage::TimeFormat => vec![
                ("HH:MM", None, !state.preferences.show_seconds),
                ("HH:MM:SS", None, state.preferences.show_seconds),
            ],
            Subpage::Input(id) => {
                let current = state
                    .preferences
                    .input_modes
                    .get(id)
                    .copied()
                    .unwrap_or_default();
                vec![
                    (
                        language.t("stateOn"),
                        Some(language.t("inputAlways")),
                        current == InputMode::On,
                    ),
                    (
                        language.t("stateOff"),
                        Some(language.t("inputNever")),
                        current == InputMode::Off,
                    ),
                    (
                        language.t("stateAuto"),
                        Some(language.t("inputWhenConnected")),
                        current == InputMode::Auto,
                    ),
                ]
            }
            Subpage::Temperature => vec![
                (
                    language.t("celsius"),
                    None,
                    state.preferences.temperature_unit == Unit::Celsius,
                ),
                (
                    language.t("fahrenheit"),
                    None,
                    state.preferences.temperature_unit == Unit::Fahrenheit,
                ),
            ],
            Subpage::WeatherLocation
            | Subpage::Weather
            | Subpage::Shortcuts
            | Subpage::Inputs
            | Subpage::Greeting
            | Subpage::Date
            | Subpage::Clock
            | Subpage::HiddenApps => {
                unreachable!()
            }
            Subpage::Language => Language::ALL
                .into_iter()
                .map(|option| (option.name(), None, option == language))
                .collect(),
            Subpage::Appearance => crate::model::Appearance::ALL
                .into_iter()
                .map(|mode| {
                    (
                        language.t(mode.label_key()),
                        None,
                        mode == state.preferences.appearance,
                    )
                })
                .collect(),
            Subpage::Screensaver => {
                let labels: Vec<_> = crate::state::SCREENSAVER_MINUTES
                    .iter()
                    .map(|minutes| screensaver_label(language, *minutes))
                    .collect();
                return screensaver_choices(out, state, assets, labels);
            }
        };
        for (index, (title, description, selected)) in options.into_iter().enumerate() {
            let h = if description.is_some() {
                DETAIL_ROW_H
            } else {
                ROW_H
            };
            row(
                out,
                state,
                assets,
                Row {
                    target: Target::Choice(index),
                    rect: settings_rect(SETTINGS_TOP + index as f32 * (h + ROW_GAP) - s, h),
                    title,
                    icon: None,
                    value: None,
                    description,
                    selected,
                    chevron: false,
                },
            );
        }
        if let Subpage::Input(id) = subpage {
            input_name_rows(
                out,
                state,
                assets,
                id,
                SETTINGS_TOP + 3. * (DETAIL_ROW_H + ROW_GAP) + 16. - s,
            );
        }
        return;
    }
    match state.section {
        0 => {
            let hidden = state.hidden_apps().len().to_string();
            let screensaver = screensaver_label(language, state.preferences.screensaver_minutes);
            let focus_size = format!("{}%", state.preferences.focus_grow.0);
            for (position, (index, key, icon, value, chevron)) in [
                (
                    0,
                    "appNames",
                    "type",
                    Some(if state.preferences.show_app_names {
                        language.t("stateOn")
                    } else {
                        language.t("stateOff")
                    }),
                    false,
                ),
                (1, "clock", "clock", None, true),
                (4, "language", "language", Some(language.name()), true),
                (11, "hiddenApps", "eye-off", Some(hidden.as_str()), true),
                (
                    14,
                    "appearance",
                    "weather-sun",
                    Some(language.t(state.preferences.appearance.label_key())),
                    true,
                ),
                (
                    12,
                    "screensaver",
                    "weather-moon",
                    Some(screensaver.as_str()),
                    true,
                ),
                // Temporary: settling the focus size on the TV. OK cycles 6% to 14%.
                (
                    16,
                    "focusGrow",
                    "sparkles",
                    Some(focus_size.as_str()),
                    false,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                row(
                    out,
                    state,
                    assets,
                    Row {
                        target: Target::Setting(index),
                        rect: settings_rect(row_y(position) - s, ROW_H),
                        title: language.t(key),
                        icon: Some(icon),
                        value,
                        description: None,
                        selected: false,
                        chevron,
                    },
                );
            }
        }
        1 => {
            for index in 0..state.background_count() {
                let id = state.background_id(index).unwrap();
                let custom = state.wallpapers.iter().find(|item| item.id == id);
                let rect = gallery_rect(index, s);
                let target = Target::Background(index);
                let level = state.display_level(&target);
                let radius = CARD_RADIUS;
                let drawn = focus_card(out, rect, radius, level);
                out.image(
                    custom.map_or_else(
                        || assets.path(format!("thumbnails/{id}.jpg")),
                        |item| item.thumbnail(),
                    ),
                    drawn,
                    radius,
                    false,
                );
                card_glass(out, drawn, radius, level);
                if state.preferences.background == id {
                    // Selected: a solid white edge and check badge, at any focus level.
                    out.border(drawn, radius, 3., c(0xffffff, 0.95));
                    let check =
                        Rect::new(drawn.x + drawn.w - 46., drawn.y + drawn.h - 46., 32., 32.);
                    out.shadow(check, 16., 8., 2., 0.25);
                    out.shape(check, 16., c(0xffffff, 1.));
                    out.icon(assets, "check", check.inset(7.), ON_LIGHT);
                }
                out.control(target, rect, true);
            }
        }
        WIDGETS_SECTION => {
            for (index, (widget, key, icon)) in [
                (4, "date", "clock"),
                (0, "greeting", "type"),
                (1, "weather", "weather-sun"),
                (2, "shortcuts", "sparkles"),
                (3, "inputs", "input-hdmi"),
            ]
            .into_iter()
            .enumerate()
            {
                row(
                    out,
                    state,
                    assets,
                    Row {
                        target: Target::Widget(widget),
                        rect: settings_rect(row_y(index) - s, ROW_H),
                        title: language.t(key),
                        icon: Some(icon),
                        value: None,
                        description: None,
                        selected: false,
                        chevron: true,
                    },
                );
            }
        }
        SYSTEM_SECTION => {
            let rect = settings_rect(SETTINGS_TOP - s, 256.);
            let title = if matches!(state.update, crate::update::InstallStatus::Failed(_)) {
                "updateError"
            } else if state.available_update.is_some() {
                "updateAvailable"
            } else {
                match state.update_status {
                    crate::update::Status::Unchecked => "checkForUpdates",
                    crate::update::Status::Checking => "checkingUpdates",
                    crate::update::Status::Checked => "latestVersionInstalled",
                    crate::update::Status::Failed => "updateCheckFailed",
                }
            };
            out.glass(rect, ROW_RADIUS + 8., Glass::Regular);
            out.text(
                language.t(title),
                Rect::new(rect.x + ROW_PAD, rect.y + 28., rect.w - 2. * ROW_PAD, 38.),
                CALLOUT.weight(600),
                WHITE,
                false,
            );
            out.text(
                if let Some(latest) = &state.available_update {
                    language.format(
                        "updateVersions",
                        &[("current", assets.version.trim()), ("latest", latest)],
                    )
                } else {
                    language.format("currentVersion", &[("current", assets.version.trim())])
                },
                Rect::new(rect.x + ROW_PAD, rect.y + 74., rect.w - 2. * ROW_PAD, 36.),
                BODY.weight(400),
                SECONDARY,
                false,
            );
            row(
                out,
                state,
                assets,
                Row {
                    target: if state.available_update.is_some() {
                        Target::InstallUpdate
                    } else {
                        Target::CheckUpdates
                    },
                    rect: Rect::new(rect.x + 20., rect.y + 136., rect.w - 40., ROW_H),
                    title: &update_value(state),
                    icon: Some(if state.available_update.is_some() {
                        "arrow"
                    } else {
                        "refresh"
                    }),
                    value: None,
                    description: None,
                    selected: false,
                    chevron: false,
                },
            );
            for (position, (index, title, icon, value, chevron)) in [
                (
                    3,
                    language.t(if state.loading {
                        "refreshNotice"
                    } else {
                        "refreshApps"
                    }),
                    "refresh",
                    None,
                    false,
                ),
                (5, language.t("quitApp"), "power", None, false),
            ]
            .into_iter()
            .enumerate()
            {
                row(
                    out,
                    state,
                    assets,
                    Row {
                        target: Target::Setting(index),
                        rect: settings_rect(
                            SETTINGS_TOP
                                + 256.
                                + ROW_GAP * 2.
                                + position as f32 * (ROW_H + ROW_GAP)
                                - s,
                            ROW_H,
                        ),
                        title,
                        icon: Some(icon),
                        value,
                        description: None,
                        selected: false,
                        chevron,
                    },
                );
            }
        }
        _ => {}
    }
}

/// Background thumbnails: three 16:9 cards per row across the content column.
pub const GALLERY_WIDTH: f32 = 380.;
pub const GALLERY_HEIGHT: f32 = 214.;
const GALLERY_GAP_X: f32 = (SETTINGS_W - 3. * GALLERY_WIDTH) / 2.;
const GALLERY_GAP_Y: f32 = 44.;

pub fn gallery_rect(index: usize, scroll: f32) -> Rect {
    Rect::new(
        SETTINGS_X + (index % 3) as f32 * (GALLERY_WIDTH + GALLERY_GAP_X),
        SETTINGS_TOP + (index / 3) as f32 * (GALLERY_HEIGHT + GALLERY_GAP_Y) - scroll,
        GALLERY_WIDTH,
        GALLERY_HEIGHT,
    )
}

/// Upload: a fixed glass button on the title line, above the gallery.
fn background_actions(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let upload_label = language.t("uploadWallpaper");
    let upload_width = (assets.width(upload_label, BODY) + ROW_PAD * 2. + 50.)
        .ceil()
        .min(520.);
    let right = SETTINGS_X + SETTINGS_W;
    row(
        out,
        state,
        assets,
        Row {
            target: Target::UploadWallpaper,
            rect: Rect::new(
                right - upload_width,
                SETTINGS_TITLE_Y,
                upload_width,
                TITLE_LINE_H,
            ),
            title: upload_label,
            icon: Some("upload"),
            value: None,
            description: None,
            selected: false,
            chevron: false,
        },
    );
}

fn inputs_settings(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let s = state.scroll;
    // Whether input cards appear on the main screen; inputs are otherwise on the remote.
    row(
        out,
        state,
        assets,
        Row {
            target: Target::InputsToggle,
            rect: settings_rect(row_y(0) - s, ROW_H),
            title: language.t("showWidget"),
            icon: Some("eye"),
            value: Some(language.t(if state.preferences.show_inputs {
                "stateOn"
            } else {
                "stateOff"
            })),
            description: None,
            selected: false,
            chevron: false,
        },
    );
    let top = row_y(1);
    for (index, input) in state.inventory.inputs.iter().enumerate() {
        let mode = state
            .preferences
            .input_modes
            .get(&input.id)
            .copied()
            .unwrap_or_default();
        row(
            out,
            state,
            assets,
            Row {
                target: Target::InputMode(input.id.clone()),
                rect: settings_rect(
                    top + index as f32 * (DETAIL_ROW_H + ROW_GAP) - s,
                    DETAIL_ROW_H,
                ),
                title: state.preferences.input_title(input),
                icon: Some(&format!(
                    "input-{}",
                    input_icon(input, state.preferences.input_name(input))
                )),
                value: Some(language.t(match mode {
                    InputMode::On => "stateOn",
                    InputMode::Off => "stateOff",
                    InputMode::Auto => "stateAuto",
                })),
                description: Some(&input_status(
                    input,
                    state.preferences.input_name(input).is_some(),
                    language,
                )),
                selected: false,
                chevron: true,
            },
        );
    }
    let mut y = top + state.inventory.inputs.len() as f32 * (DETAIL_ROW_H + ROW_GAP) + 16. - s;
    if state.inventory.inputs.is_empty() {
        out.text(
            language.t("noInputs"),
            Rect::new(
                SETTINGS_X + ROW_PAD,
                top - s,
                SETTINGS_W - 2. * ROW_PAD,
                36.,
            ),
            BODY.weight(400),
            SECONDARY,
            false,
        );
        y += 48.;
    }
    row(
        out,
        state,
        assets,
        Row {
            target: Target::Setting(3),
            rect: settings_rect(y, ROW_H),
            title: if state.loading {
                language.t("refreshNotice")
            } else {
                language.t("refreshInputs")
            },
            icon: Some("refresh"),
            value: None,
            description: None,
            selected: false,
            chevron: false,
        },
    );
}

fn greeting_settings(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let description = if state.text_input_active() && state.keyboard_field == TextField::Name {
        format!("{}|", state.greeting_name_draft)
    } else if state.preferences.greeting_name.is_empty() {
        language.t("stateOff").into()
    } else {
        state.preferences.greeting_name.clone()
    };
    row(
        out,
        state,
        assets,
        Row {
            target: Target::GreetingName,
            rect: settings_rect(row_y(1) - state.scroll, DETAIL_ROW_H),
            title: language.t("name"),
            icon: Some("type"),
            value: None,
            description: Some(&description),
            selected: false,
            chevron: true,
        },
    );
    // Always present so completing name entry cannot move a clicked control.
    row(
        out,
        state,
        assets,
        Row {
            target: Target::GreetingNameClear,
            rect: settings_rect(row_y(1) + DETAIL_ROW_H + ROW_GAP - state.scroll, ROW_H),
            title: language.t("clearName"),
            icon: Some("refresh"),
            value: None,
            description: None,
            selected: false,
            chevron: false,
        },
    );
    row(
        out,
        state,
        assets,
        Row {
            target: Target::GreetingToggle,
            rect: settings_rect(row_y(0) - state.scroll, ROW_H),
            title: language.t("showWidget"),
            icon: None,
            value: Some(language.t(if state.preferences.show_greeting {
                "stateOn"
            } else {
                "stateOff"
            })),
            description: None,
            selected: false,
            chevron: false,
        },
    );
}

fn date_settings(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    for (index, (target, title, value, chevron)) in [
        (
            Target::DateToggle,
            "showWidget",
            language.t(if state.preferences.show_date {
                "stateOn"
            } else {
                "stateOff"
            }),
            false,
        ),
        (
            Target::DateFormat,
            "dateFormat",
            state.preferences.date_format.label(language),
            true,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        row(
            out,
            state,
            assets,
            Row {
                target,
                rect: settings_rect(row_y(index) - state.scroll, ROW_H),
                title: language.t(title),
                icon: None,
                value: Some(value),
                description: None,
                selected: false,
                chevron,
            },
        );
    }
}

fn clock_settings(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    for (index, (target, key, value)) in [
        (
            Target::ClockFormat,
            "clockFormat",
            language.t(if state.preferences.uses_24_hour() {
                "hour24"
            } else {
                "hour12"
            }),
        ),
        (
            Target::ClockTimeFormat,
            "timeFormat",
            if state.preferences.show_seconds {
                "HH:MM:SS"
            } else {
                "HH:MM"
            },
        ),
    ]
    .into_iter()
    .enumerate()
    {
        row(
            out,
            state,
            assets,
            Row {
                target,
                rect: settings_rect(row_y(index) - state.scroll, ROW_H),
                title: language.t(key),
                icon: None,
                value: Some(value),
                description: None,
                selected: false,
                chevron: true,
            },
        );
    }
}

fn input_name_rows(out: &mut Scene, state: &State, assets: &Assets, id: &str, y: f32) {
    let language = state.preferences.language;
    let custom = state.preferences.input_names.get(id);
    let editing = state.text_input_active() && state.keyboard_field == TextField::InputName;
    let description = if editing {
        format!("{}|", state.input_name)
    } else {
        custom
            .cloned()
            .unwrap_or_else(|| language.t("inputNameHint").into())
    };
    row(
        out,
        state,
        assets,
        Row {
            target: Target::InputRename,
            rect: settings_rect(y, DETAIL_ROW_H),
            title: language.t("renameInput"),
            icon: Some("type"),
            value: None,
            description: Some(&description),
            selected: false,
            chevron: false,
        },
    );
    if custom.is_some() {
        row(
            out,
            state,
            assets,
            Row {
                target: Target::InputNameReset,
                rect: settings_rect(y + DETAIL_ROW_H + ROW_GAP, ROW_H),
                title: language.t("clearName"),
                icon: Some("refresh"),
                value: None,
                description: None,
                selected: false,
                chevron: false,
            },
        );
    }
}

fn shortcut_settings(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let tiles = &state.preferences.custom_tiles;
    for (index, tile) in tiles.iter().enumerate() {
        let description = format!("{} · {}", tile.method.label(), shorten(&tile.url));
        let y = row_y(1) + index as f32 * (DETAIL_ROW_H + ROW_GAP) - state.scroll;
        row(
            out,
            state,
            assets,
            Row {
                target: Target::Tile(tile.id.clone()),
                rect: settings_rect(y, DETAIL_ROW_H),
                title: &tile.title,
                icon: Some(tile.icon()),
                value: None,
                description: Some(&description),
                selected: false,
                chevron: false,
            },
        );
    }
    if tiles.is_empty() {
        let rect = settings_rect(row_y(1) - state.scroll, 240.);
        out.glass(rect, ROW_RADIUS + 8., Glass::Regular);
        out.icon(
            assets,
            "sparkles",
            Rect::new(rect.x + (rect.w - 40.) / 2., rect.y + 28., 40., 40.),
            SECONDARY,
        );
        out.text(
            language.t("noTiles"),
            Rect::new(rect.x + 32., rect.y + 84., rect.w - 64., 38.),
            CALLOUT.weight(600),
            WHITE,
            true,
        );
        out.text(
            language.t("addFirstShortcutHint"),
            Rect::new(rect.x + 80., rect.y + 134., rect.w - 160., 64.),
            CAPTION1.weight(400),
            SECONDARY,
            true,
        );
    }
    row(
        out,
        state,
        assets,
        Row {
            target: Target::TileEditor,
            rect: settings_rect(row_y(0) - state.scroll, ROW_H),
            title: language.t("configure"),
            icon: Some("settings"),
            value: None,
            description: None,
            selected: false,
            chevron: false,
        },
    );
}

/// URLs have no break opportunities, so long ones are cut to a single line.
fn shorten(text: &str) -> String {
    const MAX: usize = 96;
    let count = text.chars().count();
    if count <= MAX {
        text.into()
    } else {
        format!("{}…", text.chars().take(MAX - 1).collect::<String>())
    }
}

/// A custom name such as "PS5" can choose the icon; otherwise the TV's label.
fn input_icon(input: &Input, custom: Option<&str>) -> &'static str {
    custom
        .map(device_icon)
        .filter(|icon| *icon != "hdmi")
        .unwrap_or_else(|| device_icon(&input.title))
}

fn device_icon(name: &str) -> &'static str {
    let name = name.to_lowercase().replace(' ', "");
    if name.contains("playstation") || ["ps1", "ps2", "ps3", "ps4", "ps5"].contains(&name.as_str())
    {
        "playstation"
    } else if name.contains("appletv") {
        "apple-tv"
    } else if name.contains("usb") {
        "usb"
    } else if name.contains("switch2") {
        "switch2"
    } else if name.contains("switch") {
        "switch"
    } else if name.contains("shield") {
        "shield"
    } else if name.contains("xbox") {
        "xbox"
    } else if name.contains("console") {
        "console"
    } else if ["pc", "computer", "laptop"]
        .iter()
        .any(|part| name.contains(part))
    {
        "computer"
    } else if ["stream", "fire", "roku"]
        .iter()
        .any(|part| name.contains(part))
    {
        "streamer"
    } else {
        "hdmi"
    }
}
/// Detail under an input title: the TV's own label (when connected and not just
/// the port), the port when a custom name replaces it, then the status.
pub fn input_status(input: &Input, renamed: bool, language: Language) -> String {
    let mut parts = vec![];
    if input.connected == Some(true) && input.title != input.port {
        parts.push(input.title.as_str());
    }
    if renamed {
        parts.push(input.port.as_str());
    }
    parts.push(match input.connected {
        Some(false) => language.t("noDevice"),
        None => language.t("externalInput"),
        Some(true) if input.signal == Some(false) => language.t("noSignal"),
        Some(true) => language.t("connected"),
    });
    parts.join(" · ")
}

struct EmptyState<'a> {
    rect: Rect,
    icon: &'a str,
    title: &'a str,
    hint: &'a str,
    action: Option<Target>,
}
fn empty(out: &mut Scene, state: &State, assets: &Assets, content: EmptyState<'_>) {
    let EmptyState {
        rect,
        icon,
        title,
        hint,
        action,
    } = content;
    out.glass(rect, 24., Glass::Regular);
    let center = rect.x + rect.w / 2.;
    let top = rect.y
        + if action.is_some() {
            28.
        } else {
            (rect.h - 168.) / 2.
        };
    out.icon(
        assets,
        icon,
        Rect::new(center - 22., top, 44., 44.),
        SECONDARY,
    );
    out.text(
        title,
        Rect::new(rect.x + 32., top + 58., rect.w - 64., 38.),
        CALLOUT.weight(600),
        WHITE,
        true,
    );
    out.text(
        hint,
        Rect::new(center - 420., top + 102., 840., 64.),
        CAPTION1.weight(400),
        SECONDARY,
        true,
    );
    if let Some(target) = action {
        let label = state.preferences.language.t(if target == Target::Retry {
            "retry"
        } else {
            "exploreApps"
        });
        let width = assets.width(label, BODY.weight(600)) + 96.;
        let button = Rect::new(center - width / 2., top + 172., width, 60.);
        let level = state.focus_level(&target);
        // The view's one prominent action: white even before it has focus.
        let drawn = out.button(button, 16., level, true, BUTTON_LIFT);
        out.text(
            label,
            Rect::new(
                drawn.x + 28.,
                drawn.y + (drawn.h - 36.) / 2.,
                drawn.w - 80.,
                36.,
            ),
            BODY.weight(600),
            ON_LIGHT,
            false,
        );
        out.icon(
            assets,
            "arrow",
            Rect::new(
                drawn.x + drawn.w - 48.,
                drawn.y + (drawn.h - 22.) / 2.,
                22.,
                22.,
            ),
            ON_LIGHT,
        );
        out.control(target, button, true);
    }
}

/// tvOS context menu: the lifted card stays in place above the dimmed, blurred stage and a
/// vertical stack of actions sits beside it on one glass panel.
fn options(out: &mut Scene, state: &State, assets: &Assets, id: &str, anchor: Option<Rect>) {
    let language = state.preferences.language;
    let shortcut = matches!(
        &state.overlay,
        Some(Overlay::Options {
            restore: Target::Tile(_),
            ..
        })
    );
    let background = matches!(state.overlay, Some(Overlay::BackgroundOptions { .. }));
    let app = state.inventory.apps.iter().find(|app| app.id == id);
    let deletable = if background {
        state.can_delete_background(id) && state.deleting_wallpaper.is_none()
    } else {
        crate::platform::can_uninstall(id)
    };
    let hidden = state.preferences.hidden.iter().any(|item| item == id);
    let actions = if shortcut {
        vec![
            (Target::TileTest(id.into()), language.t("testShortcut")),
            (Target::TileRemove(id.into()), language.t("removeTile")),
        ]
    } else if background {
        let mut actions = vec![(Target::UseBackground, language.t("useBackground"))];
        if deletable {
            actions.push((Target::DeleteBackground, language.t("deleteBackground")));
        }
        actions
    } else {
        vec![
            // Move first: a long press then OK starts moving the app.
            (Target::Option(0), language.t("moveApp")),
            (Target::OpenApp, language.t("open")),
            (
                Target::Option(1),
                language.t(if hidden { "showApp" } else { "hideApp" }),
            ),
            (Target::Option(2), language.t("deleteApp")),
        ]
        .into_iter()
        .take(if deletable { 4 } else { 3 })
        .collect()
    };
    let item_height = 76.;
    let pitch = item_height + 8.;
    let padding = 14.;
    let height = 2. * padding + actions.len() as f32 * pitch - 8.;
    let width = 580.;
    let card = anchor.unwrap_or(Rect::new(460., 380., CARD_WIDTH, CARD_HEIGHT));
    let x = if card.x + card.w + 48. + width <= 1920. - SAFE_X {
        card.x + card.w + 48.
    } else {
        (card.x - 48. - width).max(SAFE_X)
    };
    let panel = Rect::new(x, card.y.clamp(144., 1020. - height), width, height);
    // The selected card keeps its focused highlight and size step, in place.
    let lifted = focused_rect(out.focus, card);
    let radius = CARD_RADIUS;
    if background {
        let path = state
            .wallpapers
            .iter()
            .find(|item| item.id == id)
            .map_or_else(
                || assets.path(format!("thumbnails/{id}.jpg")),
                |item| item.thumbnail(),
            );
        focus_card(out, card, radius, 1.);
        out.image(path, lifted, radius, false);
        card_glass(out, lifted, radius, 1.);
    } else if let Some(app) = app.filter(|_| !shortcut) {
        focus_card(out, card, radius, 1.);
        app_face(out, assets, app, lifted, language, radius);
        card_glass(out, lifted, radius, 1.);
    }
    out.modal = Some(panel);
    out.shadow(panel, PANEL_RADIUS, 90., 26., 0.42);
    out.glass(panel, PANEL_RADIUS, Glass::Regular);
    for (row, (target, label)) in actions.into_iter().enumerate() {
        let rect = Rect::new(
            panel.x + padding,
            panel.y + padding + row as f32 * pitch,
            panel.w - 2. * padding,
            item_height,
        );
        let destructive = matches!(
            target,
            Target::Option(2) | Target::DeleteBackground | Target::TileRemove(_)
        );
        let level = state.display_level(&target);
        let drawn = grow(rect, out.lift(BUTTON_LIFT / 2.) * level);
        if level > 0. {
            out.lift_shadow(drawn, 20., level);
            out.draw.push(Draw::RoundedGradient(
                drawn,
                20.,
                c(0xffffff, level),
                c(0xebebed, level),
            ));
        }
        let color = if destructive {
            mix_color(c(0xff6961, 1.), c(0xd70015, 1.), level)
        } else {
            mix_color(WHITE, ON_LIGHT, level)
        };
        out.text(
            label,
            Rect::new(
                drawn.x + 26.,
                drawn.y + (drawn.h - 36.) / 2.,
                drawn.w - 96.,
                36.,
            ),
            BODY.weight(600),
            color,
            false,
        );
        out.icon(
            assets,
            match &target {
                Target::OpenApp | Target::TileTest(_) => "arrow",
                Target::UseBackground => "check",
                Target::Option(0) => "grid",
                Target::Option(1) => {
                    if hidden {
                        "eye"
                    } else {
                        "eye-off"
                    }
                }
                _ => "circle-minus",
            },
            Rect::new(
                drawn.x + drawn.w - 54.,
                drawn.y + (drawn.h - 26.) / 2.,
                26.,
                26.,
            ),
            color,
        );
        out.control(target, rect, false);
    }
}

/// tvOS alert: a centred glass card, the message, then the actions side by side with Cancel
/// focused by default.
fn delete_confirmation(out: &mut Scene, state: &State, assets: &Assets, id: &str) {
    let language = state.preferences.language;
    let background = matches!(state.overlay, Some(Overlay::BackgroundDelete { .. }));
    let deleting = if background {
        state.deleting_wallpaper.as_deref() == Some(id)
    } else {
        state.uninstalling.as_deref() == Some(id)
    };
    let warning_key = if background {
        "deleteBackgroundWarning"
    } else {
        "deleteAppWarning"
    };
    let deleting_key = if background {
        "deletingBackground"
    } else {
        "deletingApp"
    };
    let delete_key = if background {
        "deleteBackground"
    } else {
        "deleteApp"
    };
    let title = state
        .inventory
        .apps
        .iter()
        .find(|app| app.id == id)
        .map_or(id, |app| app.title.as_str());
    let body_style = CALLOUT.weight(600);
    let width = 880.;
    let text_width = width - 96.;
    let body = language.format(
        if deleting { deleting_key } else { warning_key },
        &[("app", title)],
    );
    // Status changes must not move the confirmation buttons under the pointer.
    let warning = language.format(warning_key, &[("app", title)]);
    let body_height = assets
        .lines(&warning, body_style, text_width)
        .max(assets.lines(language.t(deleting_key), body_style, text_width))
        .max(1) as f32
        * 38.;
    let height = 48. + body_height + 40. + 80. + 40.;
    let panel = Rect::new((1920. - width) / 2., (1080. - height) / 2., width, height);
    out.modal = Some(panel);
    out.shadow(panel, PANEL_RADIUS, 100., 28., 0.45);
    out.glass(panel, PANEL_RADIUS, Glass::Regular);
    out.text(
        body,
        Rect::new(panel.x + 48., panel.y + 48., text_width, body_height),
        body_style,
        WHITE,
        true,
    );
    let button_width = (panel.w - 40. * 2. - 24.) / 2.;
    for (index, (target, label)) in [
        (Target::ConfirmDelete, delete_key),
        (Target::CancelDelete, "cancel"),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = Rect::new(
            panel.x + 40. + index as f32 * (button_width + 24.),
            panel.y + panel.h - 40. - 80.,
            button_width,
            80.,
        );
        let disabled = target == Target::ConfirmDelete && deleting;
        let level = if disabled {
            0.
        } else {
            state.display_level(&target)
        };
        let drawn = grow(rect, out.lift(BUTTON_LIFT) * level);
        if level > 0. {
            out.lift_shadow(drawn, 20., level);
        }
        out.shape(rect, 20., c(0xffffff, if disabled { 0.05 } else { 0.12 }));
        if level > 0. {
            out.draw.push(Draw::RoundedGradient(
                drawn,
                20.,
                c(0xffffff, level),
                c(0xebebed, level),
            ));
        }
        let color = if index == 0 {
            mix_color(
                c(0xff6961, if disabled { 0.4 } else { 1. }),
                c(0xd70015, 1.),
                level,
            )
        } else {
            mix_color(WHITE, ON_LIGHT, level)
        };
        out.text(
            language.t(label),
            Rect::new(
                drawn.x + 20.,
                drawn.y + (drawn.h - 36.) / 2.,
                drawn.w - 40.,
                36.,
            ),
            assets.fit_text(language.t(label), BODY.weight(600), drawn.w - 40.),
            color,
            true,
        );
        if !disabled {
            out.control(target, rect, false);
        }
    }
}

/// tvOS's linear keyboard for the weather location: one row of keys that scrolls with focus,
/// the text above it and live location suggestions beneath it (Down reaches them, OK picks
/// one and closes the keyboard).
fn linear_keyboard(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let panel = Rect::new(SAFE_X, 230., 1920. - 2. * SAFE_X, 790.);
    out.modal = Some(panel);
    out.shadow(panel, PANEL_RADIUS, 90., 24., 0.42);
    out.glass(panel, PANEL_RADIUS, Glass::Regular);
    out.text(
        language.t("cityPostcode"),
        Rect::new(panel.x + 48., panel.y + 32., 1200., 56.),
        TITLE3,
        WHITE,
        false,
    );
    let close = Rect::new(panel.x + panel.w - 48. - 60., panel.y + 30., 60., 60.);
    let level = state.focus_level(&Target::CloseDialog);
    let drawn = out.button(close, 30., level, false, BUTTON_LIFT);
    out.icon(
        assets,
        "close",
        drawn.inset(18. * drawn.w / close.w),
        mix_color(WHITE, ON_LIGHT, level),
    );
    out.control(Target::CloseDialog, close, false);
    // The text, large, with a cursor.
    out.text(
        format!("{}|", state.editing_text()),
        Rect::new(panel.x + 48., panel.y + 112., panel.w - 96., 56.),
        TITLE3.weight(500),
        WHITE,
        false,
    );
    // One row of keys: letters and digits, then Space, Delete, Clear and Done. The row scrolls
    // so the focused key stays near the middle; keys outside the panel are not drawn.
    let label = |key: &str| match key {
        "Space" => language.t("space").to_owned(),
        "Delete" => language.t("delete").to_owned(),
        "Clear" => language.t("clear").to_owned(),
        "Done" => language.t("done").to_owned(),
        key => key.to_owned(),
    };
    let widths: Vec<f32> = KEYS
        .iter()
        .map(|key| {
            if key.len() == 1 {
                64.
            } else {
                (assets.width(&label(key), BODY.weight(600)) + 40.).max(120.)
            }
        })
        .collect();
    let gap = 12.;
    let starts: Vec<f32> = widths
        .iter()
        .scan(0., |x, width| {
            let start = *x;
            *x += width + gap;
            Some(start)
        })
        .collect();
    let total = starts.last().unwrap() + widths.last().unwrap();
    let row = Rect::new(panel.x + 40., panel.y + 212., panel.w - 80., 80.);
    let focused = match state.focus {
        Target::Key(index) => index,
        _ => 0,
    };
    let offset =
        (starts[focused] + widths[focused] / 2. - row.w / 2.).clamp(0., (total - row.w).max(0.));
    for (index, key) in KEYS.iter().enumerate() {
        let x = row.x + starts[index] - offset;
        if x < row.x - 1. || x + widths[index] > row.x + row.w + 1. {
            continue;
        }
        let rect = Rect::new(x, row.y, widths[index], row.h);
        let target = Target::Key(index);
        let level = state.display_level(&target);
        let drawn = grow(rect, out.lift(BUTTON_LIFT * 1.5) * level);
        if level > 0. {
            out.lift_shadow(drawn, 16., level);
            out.draw.push(Draw::RoundedGradient(
                drawn,
                16.,
                c(0xffffff, level),
                c(0xebebed, level),
            ));
        }
        let text = label(key);
        let style = assets.fit_text(&text, BODY.weight(600), rect.w - 12.);
        out.text(
            text,
            Rect::new(drawn.x, drawn.y + (drawn.h - 36.) / 2., drawn.w, 36.),
            style,
            mix_color(WHITE, ON_LIGHT, level),
            true,
        );
        out.control(target, rect, false);
    }
    // Suggestions as they arrive, or why there are none.
    let mut y = row.y + row.h + 40.;
    let status = if state.weather.searching {
        Some(language.t("searching"))
    } else {
        state.weather.message.as_deref().map(|key| language.t(key))
    };
    if let Some(status) = status.filter(|_| state.weather.results.is_empty()) {
        out.text(
            status,
            Rect::new(panel.x + 48., y, panel.w - 96., 36.),
            BODY.weight(400),
            SECONDARY,
            false,
        );
    }
    for (index, place) in state.weather.results.iter().enumerate().take(5) {
        let rect = Rect::new(panel.x + 40., y, panel.w - 80., 72.);
        let target = Target::WeatherResult(index);
        let level = state.focus_level(&target);
        let drawn = out.button(rect, ROW_RADIUS, level, false, ROW_LIFT);
        out.icon(
            assets,
            "location",
            Rect::new(drawn.x + 24., drawn.y + (drawn.h - 28.) / 2., 28., 28.),
            mix_color(WHITE, ON_LIGHT, level),
        );
        let text = format!("{} · {}", place.name, place.detail());
        let (text, style) = assets.single_line(&text, BODY, drawn.w - 100.);
        out.text(
            text,
            Rect::new(
                drawn.x + 70.,
                drawn.y + (drawn.h - 36.) / 2.,
                drawn.w - 100.,
                36.,
            ),
            style,
            mix_color(WHITE, ON_LIGHT, level),
            false,
        );
        out.control(target, rect, false);
        y += rect.h + 10.;
    }
}

fn keyboard(out: &mut Scene, state: &State, assets: &Assets) {
    if state.keyboard_field == TextField::Location {
        return linear_keyboard(out, state, assets);
    }
    let language = state.preferences.language;
    let panel = Rect::new(330., 250., 1260., 600.);
    out.modal = Some(panel);
    out.shadow(panel, PANEL_RADIUS, 90., 24., 0.42);
    out.glass(panel, PANEL_RADIUS, Glass::Regular);
    out.text(
        match state.keyboard_field {
            TextField::Location => language.t("cityPostcode"),
            TextField::Name => language.t("greetingName"),
            TextField::InputName => language.t("inputName"),
        },
        Rect::new(panel.x + 40., panel.y + 32., 1000., 56.),
        TITLE3,
        WHITE,
        false,
    );
    let close = Rect::new(panel.x + panel.w - 40. - 60., panel.y + 30., 60., 60.);
    let level = state.focus_level(&Target::CloseDialog);
    let drawn = out.button(close, 30., level, false, BUTTON_LIFT);
    out.icon(
        assets,
        "close",
        drawn.inset(18. * drawn.w / close.w),
        mix_color(WHITE, ON_LIGHT, level),
    );
    out.control(Target::CloseDialog, close, false);
    let entry = Rect::new(panel.x + 40., panel.y + 112., panel.w - 80., 68.);
    out.shape(entry, 34., c(0x000000, 0.28));
    out.draw.push(Draw::Rim(
        entry,
        34.,
        1.5,
        c(0xffffff, 0.06),
        c(0xffffff, 0.18),
    ));
    out.icon(
        assets,
        "search",
        Rect::new(entry.x + 26., entry.y + 21., 26., 26.),
        SECONDARY,
    );
    out.text(
        format!("{}|", state.editing_text()),
        Rect::new(entry.x + 68., entry.y + 16., entry.w - 96., 36.),
        BODY.weight(400),
        WHITE,
        false,
    );
    for (index, key) in KEYS.iter().enumerate() {
        let label = match *key {
            "Space" => language.t("space"),
            "Delete" => language.t("delete"),
            "Clear" => language.t("clear"),
            "Done" => language.t("done"),
            _ => key,
        };
        let rect = Rect::new(
            panel.x + 40. + (index % 10) as f32 * 118.,
            panel.y + 212. + (index / 10) as f32 * 88.,
            106.,
            76.,
        );
        let target = Target::Key(index);
        let level = state.display_level(&target);
        let drawn = grow(rect, out.lift(BUTTON_LIFT * 1.5) * level);
        if level > 0. {
            out.lift_shadow(drawn, 16., level);
        }
        out.shape(rect, 16., c(0xffffff, 0.1 * (1. - level)));
        if level > 0. {
            out.draw.push(Draw::RoundedGradient(
                drawn,
                16.,
                c(0xffffff, level),
                c(0xebebed, level),
            ));
        }
        let style = assets.fit_text(label, BODY.weight(600), rect.w - 12.);
        out.text(
            label,
            Rect::new(drawn.x, drawn.y + (drawn.h - 36.) / 2., drawn.w, 36.),
            style,
            mix_color(WHITE, ON_LIGHT, level),
            true,
        );
        out.control(target, rect, false);
    }
}

/// Notices as tvOS shows them: a compact glass pill in Caption 1, centred just above the bottom
/// safe-area line. It fades in and rises a few pixels (no slide under Reduced motion); a new
/// notice replaces the current one, and it dismisses itself as before.
const NOTICE_BOTTOM: f32 = 1080. - 60.;

fn toast(out: &mut Scene, state: &State, assets: &Assets, notice: &str) {
    let start = out.draw.len();
    let notice = notice.trim_end_matches(['.', '。']);
    let style = CAPTION1.weight(500);
    let width = (assets.width(notice, style).ceil() + 56.).clamp(56., 1100.);
    let text_height =
        assets.lines(notice, style, width - 48.) as f32 * style.size * style.line_height;
    let height = 20. + text_height;
    let appear = {
        state.notice.as_ref().map_or(1., |(_, opened)| {
            let t = (opened.elapsed().as_secs_f32() / 0.24).min(1.);
            1. - (1. - t).powi(3)
        })
    };
    let settled = Rect::new(960. - width / 2., NOTICE_BOTTOM - height, width, height);
    let rect = Rect::new(settled.x, settled.y + 10. * (1. - appear), width, height);
    let radius = height.min(52.) / 2.;
    let target = Target::DismissNotice;
    let level = state.focus_level(&target);
    let drawn = out.button(rect, radius, level, false, BUTTON_LIFT / 2.);
    out.shadow(drawn, radius, 24., 8., 0.18 * (1. - level));
    out.text(
        notice,
        Rect::new(drawn.x + 24., drawn.y + 10., drawn.w - 48., text_height),
        style,
        mix_color(c(0xffffff, 0.92), ON_LIGHT, level),
        true,
    );
    // The motion is drawing only: the hit region is the settled pill from the start.
    if state.overlay.is_none() {
        out.control(target, settled, false);
    }
    if appear < 1. {
        out.fade(start, appear, 1.);
    }
}

/// A corner-less, borderless fill of the whole logical screen.
fn full_screen_fill(draw: &Draw) -> bool {
    let whole = |r: &Rect| r.x <= 0. && r.y <= 0. && r.x + r.w >= 1920. && r.y + r.h >= 1080.;
    match draw {
        Draw::Shape(rect, radius, border, _) => whole(rect) && *radius <= 0. && *border <= 0.,
        Draw::RoundedGradient(rect, radius, ..) => whole(rect) && *radius <= 0.,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Preferences, demo_inventory};

    fn assets() -> Assets {
        Assets {
            root: PathBuf::new(),
            version: "test".into(),
            metrics: BTreeMap::from([
                ("display-500".into(), BTreeMap::from([('W', 1.)])),
                ("500".into(), BTreeMap::from([('W', 1.2)])),
            ]),
            artwork: crate::app_artwork::Service::new(false),
        }
    }

    #[test]
    fn settings_breadcrumbs_use_equal_gaps_and_only_highlight_the_leaf() {
        let assets = assets();
        let mut state = State::new(Preferences::default());
        state.open(Page::Settings);
        state.section = 2;
        state.subpage = Some(Subpage::DateFormat);
        for language in Language::ALL {
            state.preferences.language = language;
            for focused in [false, true] {
                state.focus = if focused {
                    Target::Sidebar(2)
                } else {
                    Target::Setting(0)
                };
                let mut scene = Scene::new();
                settings_title(&mut scene, &state, &assets);
                let line = SETTINGS_TITLE_Y + (TITLE_LINE_H - 46.) / 2.;
                let parts: Vec<_> = scene
                    .draw
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::Text(label, rect, style, color, _)
                            if (rect.y - line).abs() < 0.001 =>
                        {
                            Some((label, rect, style, color))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0].0, language.t("widgets"));
                assert_eq!(parts[1].0, language.t("date"));
                assert_eq!(parts[2].0, language.t("dateFormat"));
                assert_eq!(parts[0].3, parts[1].3);
                assert!(parts[2].3[3] > parts[1].3[3]);
                let chevrons: Vec<_> = scene
                    .draw
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::TintedImage(_, rect, _, _, color)
                            if (rect.y - line - (46. - BREADCRUMB_CHEVRON) / 2.).abs() < 0.001 =>
                        {
                            assert_eq!(*color, TERTIARY);
                            Some(rect)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(chevrons.len(), 2);
                for (index, chevron) in chevrons.iter().enumerate() {
                    let (label, rect, style, _) = parts[index];
                    let left_gap = chevron.x - rect.x - assets.width(label, *style);
                    let right_gap = parts[index + 1].1.x - chevron.x - chevron.w;
                    assert!((left_gap - BREADCRUMB_GAP).abs() < 0.001);
                    assert!((right_gap - BREADCRUMB_GAP).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    fn upload_button_padding_tracks_the_label_in_every_language() {
        let assets = assets();
        let mut state = State::new(Preferences::default());
        state.open(Page::Settings);
        state.section = 1;
        for language in Language::ALL {
            state.preferences.language = language;
            let label = language.t("uploadWallpaper");
            let label_width = assets.width(label, BODY);
            let mut scene = Scene::new();
            settings(&mut scene, &state, &assets);
            let button = scene
                .controls
                .iter()
                .find(|control| control.target == Target::UploadWallpaper)
                .unwrap()
                .rect;
            let title = scene
                .draw
                .iter()
                .find_map(|draw| match draw {
                    Draw::Text(value, rect, ..) if value == label => Some(rect),
                    _ => None,
                })
                .unwrap();
            let right_padding = button.x + button.w - title.x - label_width;
            assert!((ROW_PAD..ROW_PAD + 1.4).contains(&right_padding));
        }
    }

    #[test]
    fn short_greetings_keep_their_original_style_and_geometry() {
        let assets = assets();
        for name in ["", "Ana"] {
            let state = State::new(Preferences {
                greeting_name: name.into(),
                show_weather: false,
                ..Preferences::default()
            });
            let mut scene = Scene::new();
            home(&mut scene, &state, &assets, OffsetDateTime::UNIX_EPOCH);
            let greeting = state.preferences.language.greeting(0, name);
            let (rect, style) = scene
                .draw
                .iter()
                .find_map(|draw| match draw {
                    Draw::Text(value, rect, style, ..) if value == &greeting => Some((rect, style)),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                (rect.x, rect.y, rect.w, rect.h),
                (SAFE_X - 3., SHELF_TOP, GREETING_WIDTH, 68.)
            );
            // tvOS Title 2, emphasized (bold) with tight tracking, in the compact shelf.
            assert_eq!(style.size, TITLE2.size);
            assert_eq!(style.weight, TITLE2.weight);
            assert_eq!(style.tracking, TITLE2.tracking);
        }
    }

    #[test]
    fn long_localized_greetings_fit_on_one_line_without_changing_the_name() {
        let assets = assets();
        let base = TITLE2;
        for language in Language::ALL {
            for hour in [2, 9, 15, 20] {
                for name in [
                    "W".repeat(40),
                    "Alexandra Catherine Elizabeth Montgomery".into(),
                ] {
                    let greeting = language.greeting(hour, &name);
                    let fitted = assets.fit_text(&greeting, base, 1500.);
                    assert!(fitted.size > 0. && fitted.size <= base.size);
                    assert!(assets.width(&greeting, fitted) < 1500.);
                    assert_eq!(assets.lines(&greeting, fitted, 1500.), 1);
                    assert!(greeting.contains(&name));
                }
            }
        }
    }

    #[test]
    fn fitting_remeasures_when_shrinking_changes_the_optical_font() {
        let assets = assets();
        let greeting = "W".repeat(100);
        let fitted = assets.fit_text(&greeting, TITLE2, 1500.);
        assert!(fitted.size < 32.);
        assert!(assets.width(&greeting, fitted) < 1500.);
        assert_eq!(assets.lines(&greeting, fitted, 1500.), 1);
    }

    #[test]
    fn shrinking_a_greeting_preserves_weather_favorites_and_control_positions() {
        let assets = assets();
        for show_weather in [false, true] {
            let mut state = State::new(Preferences {
                greeting_name: "Ana".into(),
                show_weather,
                ..Preferences::default()
            });
            state.set_inventory(demo_inventory(), None);
            let mut original = Scene::new();
            home(&mut original, &state, &assets, OffsetDateTime::UNIX_EPOCH);
            state.preferences.greeting_name = "W".repeat(40);
            let mut fitted = Scene::new();
            home(&mut fitted, &state, &assets, OffsetDateTime::UNIX_EPOCH);
            for (before, after) in original.draw.iter().zip(&fitted.draw) {
                if let (
                    Draw::Text(old, rect, old_style, ..),
                    Draw::Text(new, fitted_rect, style, ..),
                ) = (before, after)
                {
                    if old == new {
                        assert_eq!(
                            (rect.x, rect.y, rect.w, rect.h),
                            (fitted_rect.x, fitted_rect.y, fitted_rect.w, fitted_rect.h)
                        );
                        assert_eq!(old_style.size, style.size);
                    } else {
                        assert_eq!(
                            new,
                            &state
                                .preferences
                                .language
                                .greeting(0, &state.preferences.greeting_name)
                        );
                        assert!(
                            (rect.y + rect.h / 2. - fitted_rect.y - fitted_rect.h / 2.).abs()
                                < 0.001
                        );
                        assert!(style.size * style.line_height <= fitted_rect.h);
                    }
                }
            }
            assert_eq!(original.controls.len(), fitted.controls.len());
            for (before, after) in original.controls.iter().zip(&fitted.controls) {
                assert_eq!(before.target, after.target);
                assert_eq!(
                    (before.rect.x, before.rect.y, before.rect.w, before.rect.h),
                    (after.rect.x, after.rect.y, after.rect.w, after.rect.h)
                );
            }
        }
    }

    #[test]
    fn hidden_home_widgets_collapse_the_shelf_and_the_grid_keeps_the_bottom_margin() {
        let assets = assets();
        let mut state = State::new(Preferences::default());
        state.set_inventory(demo_inventory(), None);
        let first_card = |scene: &Scene| {
            scene
                .controls
                .iter()
                .find(|control| matches!(control.target, Target::App(_)))
                .unwrap()
                .rect
        };
        let weather_y = |scene: &Scene| {
            scene
                .controls
                .iter()
                .find(|control| control.target == Target::WeatherConfigure)
                .map(|control| control.rect.y)
        };
        let mut original = Scene::new();
        home(&mut original, &state, &assets, OffsetDateTime::UNIX_EPOCH);
        let original_card = first_card(&original);
        let original_weather = weather_y(&original).unwrap();
        // The date shares the clock's line, so hiding it moves nothing.
        state.preferences.show_date = false;
        let mut rendered = Scene::new();
        home(&mut rendered, &state, &assets, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(first_card(&rendered).y, original_card.y);
        // Without the greeting the weather moves up; the grid stays at the bottom margin.
        state.preferences.show_date = true;
        state.preferences.show_greeting = false;
        let mut rendered = Scene::new();
        home(&mut rendered, &state, &assets, OffsetDateTime::UNIX_EPOCH);
        assert!(
            (weather_y(&rendered).unwrap() - (original_weather - GREETING_HEIGHT)).abs() < 0.001
        );
        assert_eq!(first_card(&rendered).y, original_card.y);
        let date = state.preferences.language.date(OffsetDateTime::UNIX_EPOCH);
        let date_y = rendered
            .draw
            .iter()
            .find_map(|draw| match draw {
                Draw::Text(label, rect, ..) if *label == date => Some(rect.y),
                _ => None,
            })
            .unwrap();
        assert!(
            date_y < HEADER.y + HEADER.h,
            "The date sits on the clock's line"
        );
        // With an empty shelf the grid still ends at the bottom margin.
        state.preferences.show_weather = false;
        let mut rendered = Scene::new();
        home(&mut rendered, &state, &assets, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(weather_y(&rendered), None);
        assert_eq!(first_card(&rendered).y, GRID_Y);
    }
}
