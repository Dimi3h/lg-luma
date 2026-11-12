use crate::i18n::Language;
use crate::model::{
    App, BACKGROUNDS, CUSTOM_TILE_PREFIX, CustomTile, INPUT_NAME_LIMIT, InputMode, Inventory,
    Preferences, clean_input_name,
};
use crate::weather::{self, Unit};
use serde::Serialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub const WIDGETS_SECTION: usize = 2;
pub const SYSTEM_SECTION: usize = 3;
pub const SECTION_COUNT: usize = 4;
/// Every Settings list ends at the bottom safe-area line and fades out over this distance above
/// it; focused rows (and the last row, scrolled fully in) stay above the fade.
pub const SETTINGS_SAFE_BOTTOM: f32 = 1080. - 60.;
pub const SETTINGS_LIST_FADE: f32 = 52.;
/// The main screen's grid: six tvOS columns.
pub const APPS_COLUMNS: usize = 6;
/// Automatic inventory retries before the error shows: with the 6.5 s Luna timeout about 50 s.
pub const INVENTORY_RETRIES: u32 = 5;
pub const INVENTORY_RETRY_DELAY: Duration = Duration::from_secs(2);
/// With more rows than fit beneath the shelf, the grid scrolls only as far as needed to show
/// the focused row, and never above this line (just below the clock).
pub const GRID_TOP_LIMIT: f32 = 150.;
pub const BACKGROUND_FADE: f32 = 0.4;
/// The clock line; scrolled content is not hit-testable above it.
pub const MAIN_TOP: f32 = 120.;
/// The screensaver's clock fades out over this long before each jump and in over this long
/// after it (just opacity, eased in and out; no fade under Reduced motion).
pub const SCREENSAVER_FADE: Duration = Duration::from_millis(350);

/// The screensaver clock's opacity `elapsed` after it started: eased in over `SCREENSAVER_FADE`
/// after each jump, eased out over the same before the next, and 1 in between. Never above 1:
/// the text keeps its resting grey.
pub fn screensaver_opacity(elapsed: Duration) -> f32 {
    let step = SCREENSAVER_STEP.as_secs_f32();
    let fade = SCREENSAVER_FADE.as_secs_f32();
    let t = elapsed.as_secs_f32() % step;
    let x = (t / fade).min((step - t) / fade).clamp(0., 1.);
    x * x * (3. - 2. * x)
}

/// The screensaver's clock jumps to a new position this often (OLED burn-in).
pub const SCREENSAVER_STEP: Duration = Duration::from_secs(5);
/// Screensaver choices in minutes (0 is Off).
pub const SCREENSAVER_MINUTES: [u32; 6] = [0, 1, 5, 10, 15, 30];

/// A card of the main screen's grid (inputs, apps and the system cards).
fn is_grid_card(target: &Target) -> bool {
    matches!(target, Target::App(_) | Target::Input(_))
}

/// LG's app store: an icon in the main screen's top bar, not a card in the grid.
pub const STORE_ID: &str = "com.webos.app.discovery";

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Page {
    /// The only main screen: the top shelf above one grid of every app.
    Home,
    /// Settings, opened from the grid's Settings card.
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Target {
    InstallUpdate,
    CheckUpdates,
    Recent(String),
    RecentEmpty,
    RecentRetry,
    App(String),
    Input(String),
    /// The main screen's top-bar icons: LG's app store and Settings.
    StoreIcon,
    SettingsIcon,
    Sidebar(usize),
    Setting(usize),
    Widget(usize),
    GreetingName,
    GreetingNameClear,
    GreetingToggle,
    InputsToggle,
    DateToggle,
    DateFormat,
    ClockFormat,
    ClockTimeFormat,
    Background(usize),
    UploadWallpaper,
    UseBackground,
    /// Background options: use this wallpaper between sunset and sunrise (or stop).
    DeleteBackground,
    InputMode(String),
    InputRename,
    InputNameReset,
    Choice(usize),
    CloseDialog,
    Retry,
    DismissNotice,
    Option(usize),
    OpenApp,
    ConfirmDelete,
    CancelDelete,
    Key(usize),
    WeatherConfigure,
    WeatherToggle,
    WeatherLocation,
    WeatherTemperature,
    WeatherQuery,
    WeatherSearch,
    WeatherClear,
    WeatherResult(usize),
    Tile(String),
    TileTest(String),
    TileRemove(String),
    TileEditor,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn center(self) -> (f32, f32) {
        (self.x + self.w / 2., self.y + self.h / 2.)
    }
    pub fn inset(self, n: f32) -> Self {
        Self::new(self.x + n, self.y + n, self.w - 2. * n, self.h - 2. * n)
    }
}

/// A pointer drag of an app card on the main screen: in move mode, or started by holding the
/// pointer button on a card. Only the dragged card's drawing follows the pointer; every hit
/// region stays at its slot until the drop reorders the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub id: String,
    /// The pointer where the drag began and where it is now, in logical pixels.
    pub start: (f32, f32),
    pub at: (f32, f32),
    /// The pointer's offset inside the card, so the card follows it without jumping.
    pub grab: (f32, f32),
    /// Moved far enough (`DRAG_THRESHOLD`) to be a drag rather than a still press.
    pub active: bool,
    /// Started by holding the pointer on a card: a release without moving opens its options.
    pub from_hold: bool,
    /// The app whose slot the card drops into, highlighted while dragging.
    pub target: Option<String>,
}

/// How far the pointer moves before a press on a card becomes a drag.
pub const DRAG_THRESHOLD: f32 = 16.;

#[derive(Clone, Debug)]
pub struct Control {
    pub target: Target,
    pub rect: Rect,
    pub scrollable: bool,
}

#[derive(Clone, Debug)]
pub enum Overlay {
    Options { id: String, restore: Target },
    Delete { id: String, restore: Target },
    Recents { restore: Target },
    Keyboard,
    SystemKeyboard,
    WallpaperUpload,
    TileEditor,
    BackgroundOptions { id: String, restore: Target },
    BackgroundDelete { id: String, restore: Target },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Subpage {
    Greeting,
    Date,
    DateFormat,
    Weather,
    Shortcuts,
    Inputs,
    Clock,
    ClockFormat,
    TimeFormat,
    Input(String),
    WeatherLocation,
    Temperature,
    Language,
    /// Settings › General › Hidden apps: hidden apps as rows, OK shows one again.
    HiddenApps,
    /// Settings › General › Screensaver: Off or the idle minutes.
    Screensaver,
    /// Settings › General › Appearance: Dark, Light or Automatic.
    Appearance,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum TextField {
    Location,
    Name,
    InputName,
}

#[derive(Clone, Copy)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    Save,
    /// Read the app inventory again, keeping decoded icons of unchanged apps.
    Refresh,
    /// Refresh apps: read the inventory and every app icon again.
    Reload,
    Launch(String),
    Uninstall(String),
    DeleteWallpaper(String),
    Recent {
        generation: u64,
        command: crate::recents::Command,
    },
    /// Fire-and-forget HTTP request for a custom tile, off the SDL thread.
    FireWebhook(CustomTile),
    Quit,
    Weather(weather::Command),
    UpdateDownload(String),
}

pub const KEYS: [&str; 40] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "Space",
    "Delete", "Clear", "Done",
];

pub struct State {
    pub page: Page,
    pub focus: Target,
    pub inventory: Inventory,
    pub preferences: Preferences,
    pub section: usize,
    pub subpage: Option<Subpage>,
    pub scroll: f32,
    pub overlay: Option<Overlay>,
    pub moving: Option<String>,
    /// A card being dragged with the pointer (Magic Remote).
    pub drag: Option<Drag>,
    pub loading: bool,
    /// Set by Refresh apps: the next inventory also reloads every app icon.
    pub reload_artwork: bool,
    pub launching: bool,
    pub uninstalling: Option<String>,
    pub notice: Option<(String, Instant)>,
    pub error: Option<String>,
    pub available_update: Option<String>,
    pub update_status: crate::update::Status,
    pub update_check_requested: bool,
    /// Inventory loads that failed in a row while no apps were shown yet.
    pub inventory_failures: u32,
    pub dirty: bool,
    pub pointer: bool,
    pub pressed: Option<Target>,
    pub previous_background: Option<(String, Instant)>,
    pub background_fade: f32,
    pub wallpapers: Vec<crate::wallpapers::Wallpaper>,
    pub deleting_wallpaper: Option<String>,
    pub wallpaper_upload: Option<Result<crate::wallpapers::Session, String>>,
    pub tile_editor: Option<Result<crate::companion::Session, String>>,
    /// Luma update (Settings → System), driven by the update worker.
    pub update: crate::update::InstallStatus,
    pub weather: weather::Weather,
    pub keyboard_field: TextField,
    pub greeting_name_draft: String,
    pub input_name: String,
    pub recents: crate::recents::Recents,
    /// Display-only easing of focus and scrolling.
    pub motion: crate::motion::Motion,
    /// Live location suggestions: the query last seen while typing and when to search it.
    pub suggested_query: String,
    pub suggest_at: Option<Instant>,
    /// The last remote key, pointer movement or text input, for the screensaver's idle timer.
    pub last_input: Instant,
    /// The running screensaver: when it started, the seed of its clock positions, and the
    /// position step last drawn (the clock jumps every `SCREENSAVER_STEP`).
    pub screensaver: Option<(Instant, u64)>,
    pub screensaver_step: u64,
    /// Whether the last tick saw the clock mid-fade, so the frame that ends a fade is drawn.
    screensaver_fading: bool,
}

impl State {
    pub fn new(preferences: Preferences) -> Self {
        Self {
            page: Page::Home,
            focus: Target::SettingsIcon,
            inventory: Inventory::default(),
            preferences,
            section: 0,
            subpage: None,
            scroll: 0.,
            overlay: None,
            moving: None,
            drag: None,
            loading: false,
            reload_artwork: false,
            launching: false,
            uninstalling: None,
            notice: None,
            error: None,
            available_update: None,
            update_status: crate::update::Status::Unchecked,
            update_check_requested: false,
            inventory_failures: 0,
            dirty: true,
            pointer: false,
            pressed: None,
            previous_background: None,
            background_fade: BACKGROUND_FADE,
            wallpapers: vec![],
            deleting_wallpaper: None,
            wallpaper_upload: None,
            tile_editor: None,
            update: crate::update::InstallStatus::default(),
            weather: weather::Weather::default(),
            keyboard_field: TextField::Location,
            greeting_name_draft: String::new(),
            input_name: String::new(),
            recents: crate::recents::Recents::default(),
            motion: crate::motion::Motion::default(),
            suggested_query: String::new(),
            suggest_at: None,
            last_input: Instant::now(),
            screensaver: None,
            screensaver_step: 0,
            screensaver_fading: false,
        }
    }

    /// Night at the weather location: between today's sunset and sunrise from the forecast,
    /// or between 21:00 and 07:00 without one.
    pub fn is_night(&self, now: time::OffsetDateTime) -> bool {
        let minutes = now.hour() as u16 * 60 + now.minute() as u16;
        let [sunrise, sunset] = self
            .preferences
            .weather_location
            .as_ref()
            .and_then(|place| self.weather.reading(place))
            .and_then(|reading| reading.sun)
            .unwrap_or([7 * 60, 21 * 60]);
        minutes < sunrise || minutes >= sunset
    }

    /// Whether the light appearance applies now.
    pub fn light(&self, now: time::OffsetDateTime) -> bool {
        match self.preferences.appearance {
            crate::model::Appearance::Dark => false,
            crate::model::Appearance::Light => true,
            crate::model::Appearance::Automatic => !self.is_night(now),
        }
    }

    /// While the location keyboard is open, searches what was typed once typing pauses
    /// (350 ms), so suggestions appear beneath the keys. Returns the search to run, if due.
    pub fn suggest_locations(&mut self, now: Instant) -> Option<weather::Command> {
        let typing = matches!(self.overlay, Some(Overlay::Keyboard))
            && self.keyboard_field == TextField::Location;
        if !typing {
            self.suggest_at = None;
            self.suggested_query.clear();
            return None;
        }
        if self.weather.query != self.suggested_query {
            self.suggested_query = self.weather.query.clone();
            self.suggest_at = Some(now + Duration::from_millis(350));
        }
        if self.suggest_at.is_some_and(|at| now >= at) {
            self.suggest_at = None;
            if self.weather.query.trim().chars().count() >= 2 {
                self.dirty = true;
                return self.weather.search(self.preferences.language);
            }
        }
        None
    }

    /// How long Luma waits without input before the screensaver, when it is on.
    pub fn screensaver_after(&self) -> Option<Duration> {
        let minutes = self.preferences.screensaver_minutes;
        (minutes > 0).then(|| Duration::from_secs(minutes as u64 * 60))
    }

    /// Starts the clock screensaver (the idle timer ran out), at a random first position.
    pub fn start_screensaver(&mut self) {
        if self.screensaver.is_none() {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos() as u64);
            self.screensaver = Some((Instant::now(), seed));
            self.screensaver_step = 0;
            self.dirty = true;
        }
    }

    /// Which position the screensaver's clock is at: a new one every `SCREENSAVER_STEP`.
    pub fn screensaver_position_step(&self) -> Option<(u64, u64)> {
        self.screensaver.map(|(started, seed)| {
            let step = started.elapsed().as_millis() / SCREENSAVER_STEP.as_millis();
            (seed, step as u64)
        })
    }

    /// The screensaver clock's current opacity (1 when it is not fading or not running).
    pub fn screensaver_alpha(&self) -> f32 {
        self.screensaver
            .map_or(1., |(started, _)| screensaver_opacity(started.elapsed()))
    }

    /// Whether the clock is fading out or in around a jump: frames are drawn only then (and at
    /// a new minute), so the rest of the time stays idle.
    fn screensaver_fading(&self) -> bool {
        self.screensaver.is_some() && self.screensaver_alpha() < 1.
    }

    /// How long until the clock's next fade-out begins, so the loop wakes in time for it.
    pub fn screensaver_wake(&self) -> Option<Duration> {
        let (started, _) = self.screensaver?;
        let step = SCREENSAVER_STEP.as_millis();
        let into = started.elapsed().as_millis() % step;
        let fade_out = step - SCREENSAVER_FADE.as_millis();
        Some(Duration::from_millis(fade_out.saturating_sub(into) as u64))
    }

    /// Any key or pointer movement ends the screensaver (and only does that).
    pub fn stop_screensaver(&mut self) -> bool {
        self.last_input = Instant::now();
        let running = self.screensaver.take().is_some();
        self.dirty |= running;
        running
    }

    fn motion_inputs(&self) -> crate::motion::Inputs<'_> {
        crate::motion::Inputs {
            focus: &self.focus,
            page: self.page,
            section: self.section,
            subpage: &self.subpage,
            scroll: self.scroll,
            pointer: self.pointer,
            overlay: self.overlay.is_some() || self.recents_open(),
        }
    }

    /// Registers the current focus before a scene is composed (see `Motion::sync`).
    pub fn sync_motion(&mut self) {
        let mut motion = std::mem::take(&mut self.motion);
        motion.sync(&self.motion_inputs());
        self.motion = motion;
    }

    /// Tests: lets display motion run `by` from now (eased values settle).
    #[cfg(test)]
    pub fn advance_motion(&mut self, by: Duration) {
        let mut motion = std::mem::take(&mut self.motion);
        let now = Instant::now();
        motion.advance(&self.motion_inputs(), now);
        let mut at = now;
        while at < now + by {
            at += Duration::from_millis(16);
            motion.advance(&self.motion_inputs(), at);
        }
        self.motion = motion;
    }

    /// How far `target` is lifted by focus, 0 at rest and 1 fully focused.
    pub fn focus_level(&self, target: &Target) -> f32 {
        self.motion.focus_level(target, &self.focus)
    }

    /// The focus level to draw `target` with. Held OK returns a control to its resting look
    /// (tvOS click feedback), except in Settings and overlays, where focus is static: the
    /// control keeps its focused look and the release acts in the same frame.
    pub fn display_level(&self, target: &Target) -> f32 {
        let static_focus =
            self.page == Page::Settings || self.overlay.is_some() || self.recents_open();
        if self.pressed.as_ref() == Some(target) && !static_focus {
            0.
        } else {
            self.focus_level(target)
        }
    }

    /// The scroll offset content is drawn with; controls keep `scroll`.
    pub fn display_scroll(&self) -> f32 {
        self.motion.scroll(self.scroll)
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
        self.dirty = true;
    }

    pub fn update_response(&mut self, result: Result<Option<String>, String>) {
        match result {
            Ok(latest) => {
                let announce = latest.is_some() && latest != self.available_update;
                self.available_update = latest;
                self.update_status = crate::update::Status::Checked;
                if announce {
                    self.notify(self.preferences.language.t("updateAvailable"));
                }
            }
            Err(error) => {
                eprintln!("Luma update check: {error}");
                self.update_status = crate::update::Status::Failed;
            }
        }
        if matches!(self.focus, Target::CheckUpdates | Target::InstallUpdate) {
            let target = if self.available_update.is_some() {
                Target::InstallUpdate
            } else {
                Target::CheckUpdates
            };
            if self.focus != target {
                self.focus = target;
                self.pressed = None;
            }
        }
        self.dirty = true;
    }

    pub fn update_busy(&self) -> bool {
        self.update_status == crate::update::Status::Checking
            || matches!(
                self.update,
                crate::update::InstallStatus::Downloading(_)
                    | crate::update::InstallStatus::Installing(_)
            )
    }

    pub fn install_response(&mut self, response: crate::update::Response) {
        use crate::update::{InstallStatus, Response};
        match response {
            Response::Progress(percent) => self.update = InstallStatus::Downloading(percent),
            Response::Installing(version) => {
                self.update = InstallStatus::Installing(version);
                self.notify(self.preferences.language.t("updateRestart"));
            }
            Response::Finished(result) => {
                self.available_update = None;
                match result {
                    Ok(()) => {
                        self.update = InstallStatus::Idle;
                        // This process may still run the old executable. Recheck rather
                        // than claim that its in-memory version was replaced.
                        self.update_status = crate::update::Status::Unchecked;
                    }
                    Err(error) => {
                        eprintln!("Luma update: {error}");
                        self.update = InstallStatus::Failed(error);
                        self.update_status = crate::update::Status::Failed;
                        self.notify(self.preferences.language.t("updateError"));
                    }
                }
                if self.focus == Target::InstallUpdate {
                    self.focus = Target::CheckUpdates;
                    self.pressed = None;
                }
            }
        }
        self.dirty = true;
    }

    pub fn tick(&mut self) {
        // The screensaver draws nothing that moves between jumps; redraw only at a jump.
        if let Some((_, step)) = self.screensaver_position_step()
            && step != self.screensaver_step
        {
            self.screensaver_step = step;
            self.dirty = true;
        }
        // Each frame of a fade, and the one that ends it at full opacity.
        let fading = self.screensaver_fading();
        if fading || self.screensaver_fading {
            self.dirty = true;
        }
        self.screensaver_fading = fading;
        let mut motion = std::mem::take(&mut self.motion);
        self.dirty |= motion.advance(&self.motion_inputs(), Instant::now());
        self.motion = motion;
        let recents_open = self.recents_open();
        if recents_open || self.recents.visibility > 0. {
            self.dirty |= self
                .recents
                .animate(recents_open, self.pointer, Instant::now());
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, start)| start.elapsed() < Duration::from_millis(240))
        {
            self.dirty = true;
        }
        if let Some((_, start)) = &self.previous_background {
            self.dirty = true;
            if start.elapsed().as_secs_f32() >= self.background_fade {
                self.previous_background = None;
            }
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, start)| start.elapsed() > Duration::from_secs(5))
        {
            self.notice = None;
            self.dirty = true;
        }
    }

    pub fn animating(&self) -> bool {
        ((self.recents_open() || self.recents.visibility > 0.)
            && (self.recents.visibility != if self.recents_open() { 1. } else { 0. }
                || self.recents.cards_animating(self.pointer)))
            || self.previous_background.is_some()
            || self.motion.animating()
            || self.screensaver_fading()
            || self
                .notice
                .as_ref()
                .is_some_and(|(_, start)| start.elapsed() < Duration::from_millis(240))
    }

    /// Home's lifecycle relaunch returns directly to Home, even from an overlay.
    pub fn return_home(&mut self) -> Vec<Effect> {
        // Commit keyboard drafts before open() clears their settings subpage.
        let pending = self.finish_text_input();
        self.recents.pending = None;
        self.recents.visibility = 0.;
        self.recents.closing_focus = None;
        self.wallpaper_upload = None;
        self.pointer = false;
        self.pressed = None;
        self.open(Page::Home);
        pending
    }

    pub fn open(&mut self, page: Page) {
        self.weather.cancel_search();
        self.page = page;
        self.scroll = 0.;
        self.overlay = None;
        self.moving = None;
        self.drag = None;
        self.subpage = None;
        if page == Page::Settings {
            self.section = 0;
        }
        self.focus = self.first_content();
        self.dirty = true;
    }

    /// The main grid's apps in the user's order (`Preferences::ordered_ids`). Hidden apps are
    /// listed in Settings › General › Hidden apps instead.
    pub fn apps(&self) -> Vec<&App> {
        let ids = self.preferences.ordered_ids(&self.inventory.apps);
        let rank: HashMap<&str, usize> = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (id.as_str(), index))
            .collect();
        let mut apps: Vec<_> = self
            .inventory
            .apps
            .iter()
            .filter(|app| !self.preferences.hidden.contains(&app.id) && app.id != STORE_ID)
            .collect();
        apps.sort_by_key(|app| rank.get(app.id.as_str()).copied().unwrap_or(usize::MAX));
        apps
    }

    /// Starts dragging the card `id` (at `card`) with the pointer at `at`. Nothing on the grid
    /// changes until the pointer moves past `DRAG_THRESHOLD`: only then does the card follow it
    /// (and a hold become move mode), so a still hold that opens the menu leaves the grid as it is.
    pub fn drag_start(&mut self, id: &str, at: (f32, f32), card: Rect, from_hold: bool) {
        if self.page != Page::Home || self.overlay.is_some() {
            return;
        }
        self.focus = Target::App(id.into());
        self.pressed = None;
        self.notice = None;
        self.drag = Some(Drag {
            id: id.into(),
            start: at,
            at,
            grab: (at.0 - card.x, at.1 - card.y),
            active: false,
            from_hold,
            target: None,
        });
        self.dirty = true;
    }

    /// The pointer moved during a drag: the card follows it, and the app card under the pointer
    /// (its slot, from `controls`) becomes the drop target.
    pub fn drag_move(&mut self, at: (f32, f32), controls: &[Control]) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        drag.at = at;
        if !drag.active && (at.0 - drag.start.0).hypot(at.1 - drag.start.1) >= DRAG_THRESHOLD {
            drag.active = true;
            self.moving = Some(drag.id.clone());
        }
        if drag.active {
            drag.target = controls.iter().find_map(|control| match &control.target {
                Target::App(id) if control.rect.contains(at.0, at.1) => Some(id.clone()),
                _ => None,
            });
        }
        self.dirty = true;
    }

    /// The pointer button was released: a drag drops the card into the target's slot (the
    /// others shift) and ends move mode; a hold that never moved opens the card's options.
    pub fn drag_end(&mut self) -> Vec<Effect> {
        let Some(drag) = self.drag.take() else {
            return vec![];
        };
        self.dirty = true;
        if !drag.active {
            if drag.from_hold {
                self.moving = None;
                self.focus = Target::App(drag.id);
                self.options();
            }
            return vec![];
        }
        self.moving = None;
        self.focus = Target::App(drag.id.clone());
        let to = drag
            .target
            .and_then(|target| self.apps().iter().position(|app| app.id == target));
        if let Some(to) = to
            && self.move_app(&drag.id, to)
        {
            return vec![Effect::Save];
        }
        vec![]
    }

    /// Moves `id` to `to` among the visible apps; hidden apps keep their places in the order.
    /// True when the order changed.
    pub fn move_app(&mut self, id: &str, to: usize) -> bool {
        let visible: Vec<String> = self.apps().iter().map(|app| app.id.clone()).collect();
        let Some(from) = visible.iter().position(|item| item == id) else {
            return false;
        };
        let to = to.min(visible.len().saturating_sub(1));
        if from == to {
            return false;
        }
        let mut moved = visible.clone();
        let item = moved.remove(from);
        moved.insert(to, item);
        let mut moved = moved.into_iter();
        let order = self
            .preferences
            .ordered_ids(&self.inventory.apps)
            .into_iter()
            .map(|old| {
                if visible.contains(&old) {
                    moved.next().unwrap_or(old)
                } else {
                    old
                }
            })
            .collect();
        self.preferences.app_order = Some(order);
        self.dirty = true;
        true
    }

    /// Stores the order once it covers every installed app: migrated on the first run, newly
    /// installed apps appended. True when it changed and needs saving.
    fn settle_app_order(&mut self) -> bool {
        if self.inventory.apps.is_empty() {
            return false;
        }
        let order = self.preferences.ordered_ids(&self.inventory.apps);
        if self.preferences.app_order.as_ref() == Some(&order) {
            return false;
        }
        self.preferences.app_order = Some(order);
        true
    }

    /// Hidden apps that are still installed, by title (Settings › General › Hidden apps).
    pub fn hidden_apps(&self) -> Vec<&App> {
        let mut apps: Vec<_> = self
            .inventory
            .apps
            .iter()
            .filter(|app| self.preferences.hidden.contains(&app.id) && app.id != STORE_ID)
            .collect();
        apps.sort_by_key(|app| app.title.to_lowercase());
        apps
    }

    /// Every focusable grid card in order: the apps, then Settings.
    pub fn grid(&self) -> Vec<Target> {
        let mut grid = Vec::new();
        grid.extend(
            self.apps()
                .into_iter()
                .map(|app| Target::App(app.id.clone())),
        );
        grid
    }

    /// Input cards shown above the apps when Settings › Widgets › Inputs is on.
    pub fn home_inputs(&self) -> Vec<&crate::model::Input> {
        if !self.preferences.show_inputs {
            return vec![];
        }
        self.inventory
            .inputs
            .iter()
            .filter(|input| self.preferences.input_visible(input))
            .collect()
    }

    pub fn content_viewport(&self) -> Rect {
        if self.page == Page::Settings {
            // Settings lists fade out beneath the title line above their first row.
            let top = crate::view::SETTINGS_TOP - 16.;
            return Rect::new(
                crate::view::SETTINGS_X - 40.,
                top,
                1920. - (crate::view::SETTINGS_X - 40.),
                1080. - top,
            );
        }
        // The grid scrolls beneath the clock; cards stay hit-testable below it.
        Rect::new(0., MAIN_TOP, 1920., 1080. - MAIN_TOP)
    }

    fn scroll_bottom(&self) -> f32 {
        let viewport = self.content_viewport();
        if self.page == Page::Settings {
            // The same for every page and subpage, so new rows are covered as well.
            return SETTINGS_SAFE_BOTTOM - SETTINGS_LIST_FADE;
        }
        viewport.y + viewport.h - {
            // The third row ends at the bottom margin; the last row's focused caption may
            // reach into that margin, as on tvOS.
            crate::view::SCREEN_MARGIN
        }
    }

    pub fn backgrounds_viewport(&self) -> Rect {
        let content = self.content_viewport();
        let top = crate::view::SETTINGS_TOP - 12.;
        Rect::new(
            crate::view::SETTINGS_X - 20.,
            top,
            crate::view::SETTINGS_W + 40.,
            content.y + content.h - top,
        )
    }

    pub fn hit_test(&self, controls: &[Control], x: f32, y: f32) -> Option<Target> {
        controls
            .iter()
            .rev()
            .find(|control| {
                let viewport = if matches!(control.target, Target::Background(_)) {
                    self.backgrounds_viewport()
                } else {
                    self.content_viewport()
                };
                control.rect.contains(x, y) && (!control.scrollable || viewport.contains(x, y))
            })
            .map(|control| control.target.clone())
    }

    /// Scrolls the page under the pointer. Returns whether the main grid moved, so a pending
    /// card press is cancelled rather than released on a different card.
    pub fn wheel_scroll(
        &mut self,
        controls: &[Control],
        position: Option<(f32, f32)>,
        _x: i32,
        y: i32,
    ) -> bool {
        let backgrounds =
            self.page == Page::Settings && self.section == 1 && self.subpage.is_none();
        if backgrounds && position.is_some_and(|(x, y)| !self.backgrounds_viewport().contains(x, y))
        {
            return false;
        }
        let bottom = self.scroll_bottom();
        let max = controls
            .iter()
            .filter(|c| c.scrollable && (!backgrounds || matches!(c.target, Target::Background(_))))
            .map(|c| c.rect.y + c.rect.h + self.scroll - bottom)
            .fold(0., f32::max);
        let previous = self.scroll;
        self.scroll = (self.scroll - y as f32 * 90.).clamp(0., max.max(0.));
        self.pointer = true;
        self.dirty = true;
        self.page == Page::Home && self.scroll != previous
    }

    /// Applies a fresh inventory. True when the app order was stored or extended (new apps) and
    /// the preferences need saving.
    pub fn set_inventory(&mut self, next: Inventory, warning: Option<String>) -> bool {
        self.inventory.apps = next.apps;
        self.sync_custom_tiles();
        let order_changed = self.settle_app_order();
        if let Some(warning) = warning {
            self.notify(warning);
        } else {
            self.inventory.inputs = next.inputs;
        }
        self.loading = false;
        self.error = None;
        self.inventory_failures = 0;
        self.dirty = true;
        self.reconcile_focus();
        order_changed
    }

    /// Replace only input status; leave installed apps, artwork and saved preferences alone.
    pub fn set_inputs(&mut self, inputs: Vec<crate::model::Input>) {
        if self.inventory.inputs == inputs {
            return;
        }
        self.inventory.inputs = inputs;
        self.dirty = true;
        self.reconcile_focus();
        let missing = |id: &str| !self.inventory.inputs.iter().any(|input| input.id == id);
        if self.page == Page::Settings
            && (matches!(&self.subpage, Some(Subpage::Input(id)) if missing(id))
                || matches!(&self.focus, Target::InputMode(id) if missing(id)))
        {
            self.subpage = Some(Subpage::Inputs);
            self.scroll = 0.;
            self.focus = self
                .inventory
                .inputs
                .first()
                .map_or(Target::InputsToggle, |input| {
                    Target::InputMode(input.id.clone())
                });
        }
    }

    /// A failed inventory load. Right after a cold boot, or when the TV is short of memory, the
    /// Homebrew service that reads the inventory can take longer than one request allows. While
    /// no apps are shown yet, the screen keeps loading and the caller retries after the returned
    /// delay; the error and its Retry button only appear once those retries are used up.
    /// Custom tiles come from preferences, not the TV, so they don't count as loaded apps.
    pub fn inventory_failed(&mut self, error: String) -> Option<Duration> {
        self.dirty = true;
        let no_tv_apps = self
            .inventory
            .apps
            .iter()
            .all(|app| app.id.starts_with(CUSTOM_TILE_PREFIX));
        if no_tv_apps && self.inventory_failures < INVENTORY_RETRIES {
            self.inventory_failures += 1;
            return Some(INVENTORY_RETRY_DELAY);
        }
        self.inventory_failures = 0;
        self.loading = false;
        self.error = Some(error);
        None
    }

    /// Custom webhook tiles are never read from the TV. They are merged into
    /// the inventory on every refresh and edit, so favoriting, hiding and
    /// search treat them like any other app.
    pub fn sync_custom_tiles(&mut self) {
        self.inventory
            .apps
            .retain(|app| !app.id.starts_with(CUSTOM_TILE_PREFIX));
        self.inventory.apps.extend(self.preferences.custom_apps());
        self.dirty = true;
    }

    pub fn reconcile_focus(&mut self) {
        if let Target::App(id) = &self.focus
            && !self.apps().iter().any(|app| &app.id == id)
        {
            self.focus = self.first_content();
        }
        if let Target::Input(id) = &self.focus
            && !self.home_inputs().iter().any(|input| &input.id == id)
            && self.page == Page::Home
        {
            self.focus = self.first_content();
        }
    }

    /// Where focus lands when a page opens: the first favourite (the grid's first card) on the
    /// main screen, General in Settings.
    pub fn first_content(&self) -> Target {
        match self.page {
            Page::Home if self.inventory_failed_empty() => Target::Retry,
            Page::Home => self
                .apps()
                .first()
                .map_or(Target::SettingsIcon, |app| Target::App(app.id.clone())),
            Page::Settings => Target::Sidebar(0),
        }
    }

    /// The inventory failed before any app was shown: the grid offers Retry beneath it.
    pub fn inventory_failed_empty(&self) -> bool {
        self.error.is_some() && self.inventory.apps.is_empty()
    }

    /// Whether `target` is in the main screen's first card row (inputs when shown, else the
    /// grid), where the top shelf stays in view.
    fn in_first_row(&self, target: &Target) -> bool {
        match target {
            // Input glyphs sit in the top bar, above the cards.
            Target::Input(_) => true,
            Target::App(_) => self
                .grid()
                .iter()
                .position(|item| item == target)
                .is_some_and(|index| index < APPS_COLUMNS),
            _ => true,
        }
    }

    /// How far the current page can scroll, from its scrollable controls.
    fn scroll_limit(&self, controls: &[Control]) -> f32 {
        let bottom = self.scroll_bottom();
        controls
            .iter()
            .filter(|c| c.scrollable)
            .map(|c| c.rect.y + c.rect.h + self.scroll - bottom)
            .fold(0., f32::max)
    }

    pub fn focus(&mut self, target: Target, controls: &[Control]) {
        self.pointer = false;
        self.focus = target;
        self.reveal_focus(controls);
        if let Target::Sidebar(section) = self.focus {
            if self.subpage == Some(Subpage::WeatherLocation) {
                self.weather.cancel_search();
            }
            self.section = section;
            self.subpage = None;
        }
        self.dirty = true;
    }

    /// Remote focus is revealed after layout changes, including hide/reorder.
    /// Pointer paths deliberately do not call this method.
    pub fn reveal_focus(&mut self, controls: &[Control]) -> bool {
        let previous = self.scroll;
        if let Some(control) = controls.iter().find(|c| c.target == self.focus) {
            if self.page == Page::Home {
                // Up to three rows fit beneath the compact shelf, so the page stays put. With
                // more, the grid scrolls only as far as the focused row needs (the shelf
                // collapses as it does) and returns when the first row has focus again.
                self.scroll = if self.in_first_row(&self.focus) || !control.scrollable {
                    0.
                } else {
                    let mut scroll = self.scroll;
                    let bottom = self.scroll_bottom();
                    if control.rect.y + control.rect.h > bottom {
                        scroll += control.rect.y + control.rect.h - bottom;
                    }
                    if control.rect.y < GRID_TOP_LIMIT {
                        scroll -= GRID_TOP_LIMIT - control.rect.y;
                    }
                    // Scroll in whole rows, so the top visible row sits just below the clock
                    // instead of half under it; the last step may leave room beneath.
                    let first = controls
                        .iter()
                        .filter(|c| c.scrollable && is_grid_card(&c.target))
                        .map(|c| c.rect.y + self.scroll)
                        .fold(f32::MAX, f32::min);
                    let align = |value: f32| {
                        let base = first - GRID_TOP_LIMIT;
                        if value <= 0. {
                            0.
                        } else {
                            base + ((value - base) / crate::view::ROW_PITCH).ceil().max(0.)
                                * crate::view::ROW_PITCH
                        }
                    };
                    align(scroll)
                        .min(align(self.scroll_limit(controls)))
                        .max(0.)
                };
            } else if matches!(self.focus, Target::Sidebar(_)) {
                self.scroll = 0.;
            } else if control.scrollable {
                let top = if matches!(self.focus, Target::Background(_)) {
                    self.backgrounds_viewport().y + 8.
                } else {
                    crate::view::SETTINGS_TOP - 12.
                };
                if control.rect.y < top {
                    self.scroll = (self.scroll + control.rect.y - top).max(0.);
                }
                let bottom = self.scroll_bottom();
                if control.rect.y + control.rect.h > bottom {
                    self.scroll += control.rect.y + control.rect.h - bottom;
                }
            }
        }
        if self.scroll != previous {
            self.dirty = true;
        }
        self.scroll != previous
    }

    /// Hover and pointer-down must not move the control before pointer-up.
    pub fn pointer_focus(&mut self, target: Target) {
        if !self.pointer {
            self.dirty = true;
        }
        self.pointer = true;
        if self.focus == target {
            return;
        }
        if let Target::Sidebar(section) = target {
            if self.subpage == Some(Subpage::WeatherLocation) {
                self.weather.cancel_search();
            }
            if self.section != section || self.subpage.is_some() {
                self.scroll = 0.;
            }
            self.section = section;
            self.subpage = None;
        }
        self.focus = target;
        self.dirty = true;
    }

    pub fn background_count(&self) -> usize {
        self.background_ids().count()
    }

    pub fn background_id(&self, index: usize) -> Option<&str> {
        self.background_ids().nth(index)
    }

    fn background_ids(&self) -> impl Iterator<Item = &str> {
        BACKGROUNDS
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| {
                *id == self.preferences.background
                    || !self
                        .preferences
                        .deleted_backgrounds
                        .iter()
                        .any(|deleted| deleted == id)
            })
            .chain(self.wallpapers.iter().map(|item| item.id.as_str()))
    }

    pub fn can_delete_background(&self, id: &str) -> bool {
        id != self.preferences.background && self.background_ids().any(|known| known == id)
    }

    fn background_target(&self, id: &str) -> Target {
        Target::Background(
            self.background_ids()
                .position(|known| known == id)
                .unwrap_or(0),
        )
    }

    pub fn wallpaper_deleted(&mut self, id: String, result: Result<(), String>) {
        if self.deleting_wallpaper.as_deref() != Some(&id) {
            return;
        }
        self.deleting_wallpaper = None;
        if let Err(error) = result {
            eprintln!("Luma background deletion: {error}");
            self.notify(self.preferences.language.t("deleteBackgroundError"));
            return;
        }
        self.finish_background_delete(&id);
    }

    fn finish_background_delete(&mut self, id: &str) {
        let index = self
            .background_ids()
            .position(|known| known == id)
            .unwrap_or(0);
        let focused = match self.focus {
            Target::Background(index) => self.background_id(index).map(str::to_owned),
            _ => None,
        };
        self.wallpapers.retain(|item| item.id != id);
        if BACKGROUNDS.iter().any(|(known, _)| *known == id) {
            self.preferences.deleted_backgrounds.push(id.to_owned());
        }
        if matches!(&self.overlay, Some(Overlay::BackgroundOptions { id: current, .. } | Overlay::BackgroundDelete { id: current, .. }) if current == id)
        {
            self.overlay = None;
            self.focus = Target::Background(index.min(self.background_count().saturating_sub(1)));
            self.pointer = false;
        } else if let Some(focused) = focused {
            self.focus = self.background_target(&focused);
        }
        self.pressed = None;
        self.notify(self.preferences.language.t("backgroundDeleted"));
    }

    pub fn set_wallpapers(&mut self, wallpapers: Vec<crate::wallpapers::Wallpaper>) {
        // A newly uploaded ID can sort before existing entries. Keep the focused
        // image, rather than moving the pointer's target to a different photograph.
        let focused_id = if let Target::Background(index) = self.focus {
            self.background_id(index).map(str::to_owned)
        } else {
            None
        };
        self.wallpapers = wallpapers;
        if let Some(
            Overlay::BackgroundOptions { id, restore } | Overlay::BackgroundDelete { id, restore },
        ) = &self.overlay
            && !self.background_ids().any(|known| known == id)
        {
            let index = if let Target::Background(index) = restore {
                *index
            } else {
                0
            };
            self.focus = Target::Background(index.min(self.background_count().saturating_sub(1)));
            self.overlay = None;
            self.pressed = None;
            self.pointer = false;
        }
        if let Some(id) = focused_id {
            let index = (0..self.background_count())
                .find(|&i| self.background_id(i) == Some(id.as_str()))
                .unwrap_or(0);
            self.focus = Target::Background(index);
        }
        self.dirty = true;
    }

    fn section_entry(&self) -> Target {
        match self.section {
            1 => Target::Background(
                (0..self.background_count())
                    .position(|index| {
                        self.background_id(index) == Some(self.preferences.background.as_str())
                    })
                    .unwrap_or(0),
            ),
            WIDGETS_SECTION => Target::Widget(4),
            SYSTEM_SECTION if self.available_update.is_some() => Target::InstallUpdate,
            SYSTEM_SECTION => Target::CheckUpdates,
            _ => Target::Setting(0),
        }
    }

    fn close_subpage(&mut self) {
        if self.subpage == Some(Subpage::WeatherLocation) {
            self.weather.cancel_search();
        }
        self.pointer = false;
        self.focus = match self.subpage.take() {
            Some(Subpage::Greeting) => Target::Widget(0),
            Some(Subpage::Date) => Target::Widget(4),
            Some(Subpage::DateFormat) => {
                self.subpage = Some(Subpage::Date);
                Target::DateFormat
            }
            Some(Subpage::Weather) => Target::Widget(1),
            Some(Subpage::Shortcuts) => Target::Widget(2),
            Some(Subpage::Inputs) => Target::Widget(3),
            Some(Subpage::Clock) => Target::Setting(1),
            Some(Subpage::ClockFormat) => {
                self.subpage = Some(Subpage::Clock);
                Target::ClockFormat
            }
            Some(Subpage::TimeFormat) => {
                self.subpage = Some(Subpage::Clock);
                Target::ClockTimeFormat
            }
            Some(Subpage::Input(id)) => {
                self.subpage = Some(Subpage::Inputs);
                Target::InputMode(id)
            }
            Some(Subpage::WeatherLocation) => {
                self.subpage = Some(Subpage::Weather);
                Target::WeatherLocation
            }
            Some(Subpage::Temperature) => {
                self.subpage = Some(Subpage::Weather);
                Target::WeatherTemperature
            }
            Some(Subpage::Language) => Target::Setting(4),
            Some(Subpage::HiddenApps) => Target::Setting(11),
            Some(Subpage::Screensaver) => Target::Setting(12),
            Some(Subpage::Appearance) => Target::Setting(14),
            None => Target::Sidebar(self.section),
        };
        self.scroll = 0.;
        self.dirty = true;
    }

    pub fn direction(&mut self, direction: Direction, controls: &[Control]) -> Vec<Effect> {
        self.pointer = false;
        self.dirty = true;
        if self.recents_open() {
            return self.recent_direction(direction);
        }
        if matches!(self.overlay, Some(Overlay::SystemKeyboard)) {
            // The TV keyboard owns its arrows until Done/Back dismisses it.
            return vec![];
        }
        if matches!(
            self.overlay,
            Some(Overlay::WallpaperUpload | Overlay::TileEditor)
        ) {
            return vec![];
        }
        if let Some(id) = self.moving.clone() {
            // One slot at a time in the six-column grid: Left/Right step through the order
            // (wrapping across rows), Up/Down move a whole row. The other cards shift.
            let Some(index) = self.apps().iter().position(|app| app.id == id) else {
                return vec![];
            };
            let columns = APPS_COLUMNS;
            let last = self.apps().len().saturating_sub(1);
            let to = match direction {
                Direction::Left => index.checked_sub(1),
                Direction::Right => (index < last).then_some(index + 1),
                Direction::Up => index.checked_sub(columns),
                Direction::Down => (index + columns <= last).then_some(index + columns),
            };
            if let Some(to) = to
                && self.move_app(&id, to)
            {
                return vec![Effect::Save];
            }
            return vec![];
        }
        if self.overlay.is_none() {
            if self.page == Page::Home
                && let Some(next) = self.home_direction(direction)
            {
                if let Some(next) = next {
                    self.focus(next, controls);
                }
                return vec![];
            }
            let setting_rows: Option<&[Target]> = match self.subpage {
                Some(Subpage::Greeting) => Some(&[
                    Target::GreetingToggle,
                    Target::GreetingName,
                    Target::GreetingNameClear,
                ]),
                Some(Subpage::Date) => Some(&[Target::DateToggle, Target::DateFormat]),
                Some(Subpage::Clock) => Some(&[Target::ClockFormat, Target::ClockTimeFormat]),
                _ => None,
            };
            if let Some(rows) = setting_rows
                && let Some(index) = rows.iter().position(|target| *target == self.focus)
            {
                match direction {
                    Direction::Left => self.close_subpage(),
                    Direction::Up => self.focus(
                        if index == 0 {
                            self.focus.clone()
                        } else {
                            rows[index - 1].clone()
                        },
                        controls,
                    ),
                    Direction::Down if index + 1 < rows.len() => {
                        self.focus(rows[index + 1].clone(), controls)
                    }
                    Direction::Right
                        if matches!(
                            self.focus,
                            Target::GreetingName
                                | Target::ClockFormat
                                | Target::ClockTimeFormat
                                | Target::DateFormat
                        ) =>
                    {
                        return self.activate();
                    }
                    _ => {}
                }
                return vec![];
            }
            if let Target::Choice(index) = self.focus {
                let count = match self.subpage {
                    Some(Subpage::ClockFormat | Subpage::TimeFormat | Subpage::Temperature) => 2,
                    Some(Subpage::Language) => Language::ALL.len(),
                    // Without hidden apps the empty state holds focus.
                    Some(Subpage::HiddenApps) => self.hidden_apps().len().max(1),
                    Some(Subpage::Screensaver) => SCREENSAVER_MINUTES.len(),
                    Some(Subpage::Appearance) => crate::model::Appearance::ALL.len(),
                    Some(Subpage::DateFormat) => crate::model::DateFormat::ALL.len(),
                    _ => 3,
                };
                match direction {
                    Direction::Left => self.close_subpage(),
                    Direction::Up => self.focus(
                        if index == 0 {
                            self.focus.clone()
                        } else {
                            Target::Choice(index - 1)
                        },
                        controls,
                    ),
                    Direction::Down
                        if index + 1 == count
                            && matches!(self.subpage, Some(Subpage::Input(_))) =>
                    {
                        self.focus(Target::InputRename, controls)
                    }
                    Direction::Down => self.focus(Target::Choice((index + 1) % count), controls),
                    Direction::Right => {}
                }
                return vec![];
            }
            if matches!(self.focus, Target::InputRename | Target::InputNameReset) {
                match direction {
                    Direction::Left => self.close_subpage(),
                    Direction::Up => self.focus(
                        if self.focus == Target::InputNameReset {
                            Target::InputRename
                        } else {
                            Target::Choice(2)
                        },
                        controls,
                    ),
                    Direction::Down
                        if self.focus == Target::InputRename
                            && controls.iter().any(|c| c.target == Target::InputNameReset) =>
                    {
                        self.focus(Target::InputNameReset, controls)
                    }
                    _ => {}
                }
                return vec![];
            }
            if let Target::Sidebar(index) = self.focus {
                match direction {
                    Direction::Right => {
                        let entry = if self.section == 1 {
                            Target::UploadWallpaper
                        } else {
                            self.section_entry()
                        };
                        self.focus(entry, controls);
                    }
                    Direction::Up => self.focus(
                        if index == 0 {
                            self.focus.clone()
                        } else {
                            Target::Sidebar(index - 1)
                        },
                        controls,
                    ),
                    Direction::Down => {
                        self.focus(Target::Sidebar((index + 1) % SECTION_COUNT), controls)
                    }
                    Direction::Left => {}
                }
                return vec![];
            }
            if (matches!(
                self.focus,
                Target::TileEditor
                    | Target::Tile(_)
                    | Target::InputMode(_)
                    | Target::WeatherToggle
                    | Target::WeatherLocation
                    | Target::WeatherTemperature
                    | Target::Widget(_)
            ) || (self.focus == Target::Setting(3) && self.subpage == Some(Subpage::Inputs)))
                && matches!(direction, Direction::Left)
            {
                self.close_subpage();
                return vec![];
            }
            if matches!(
                self.focus,
                Target::Setting(1 | 4 | 11 | 12 | 14 | 15)
                    | Target::Widget(_)
                    | Target::InputMode(_)
                    | Target::WeatherLocation
                    | Target::WeatherTemperature
            ) && matches!(direction, Direction::Right)
            {
                return self.activate();
            }
            if matches!(self.focus, Target::Setting(_) | Target::WeatherToggle)
                && matches!(direction, Direction::Right)
            {
                return vec![];
            }
            if let Target::InputMode(id) = &self.focus
                && matches!(direction, Direction::Up)
                && self
                    .inventory
                    .inputs
                    .first()
                    .is_some_and(|input| &input.id == id)
            {
                self.focus(Target::InputsToggle, controls);
                return vec![];
            }
            if self.focus == Target::InputsToggle {
                match direction {
                    Direction::Left => self.close_subpage(),
                    Direction::Down => {
                        let next = self
                            .inventory
                            .inputs
                            .first()
                            .map_or(Target::Setting(3), |input| {
                                Target::InputMode(input.id.clone())
                            });
                        self.focus(next, controls);
                    }
                    _ => {}
                }
                return vec![];
            }
            // The top rows of Settings lists have nothing above them.
            if matches!(self.focus, Target::Setting(0) | Target::WeatherToggle)
                && matches!(direction, Direction::Up)
            {
                return vec![];
            }
            if matches!(
                self.focus,
                Target::WeatherQuery | Target::WeatherSearch | Target::WeatherClear
            ) {
                match direction {
                    Direction::Left => {
                        self.close_subpage();
                        return vec![];
                    }
                    Direction::Up => {
                        self.focus(
                            if self.preferences.weather_location.is_some()
                                && self.focus != Target::WeatherClear
                            {
                                Target::WeatherClear
                            } else {
                                self.focus.clone()
                            },
                            controls,
                        );
                        return vec![];
                    }
                    Direction::Down => {
                        let target = if self.focus == Target::WeatherClear {
                            Some(Target::WeatherQuery)
                        } else if !self.weather.results.is_empty() {
                            Some(Target::WeatherResult(0))
                        } else if self.focus == Target::WeatherQuery {
                            Some(Target::WeatherSearch)
                        } else {
                            None
                        };
                        if let Some(target) = target {
                            self.focus(target, controls);
                        }
                        return vec![];
                    }
                    _ => {}
                }
            }
            if let Target::WeatherResult(index) = self.focus {
                match direction {
                    Direction::Left => self.close_subpage(),
                    Direction::Up => self.focus(
                        if index == 0 {
                            Target::WeatherQuery
                        } else {
                            Target::WeatherResult(index - 1)
                        },
                        controls,
                    ),
                    Direction::Down if index + 1 < self.weather.results.len() => {
                        self.focus(Target::WeatherResult(index + 1), controls)
                    }
                    _ => {}
                }
                return vec![];
            }
            if let Target::Background(index) = self.focus {
                match direction {
                    Direction::Right if index + 1 < self.background_count() => {
                        self.focus(Target::Background(index + 1), controls);
                        return vec![];
                    }
                    Direction::Right => return vec![],
                    Direction::Left if index % 3 != 0 => {
                        self.focus(Target::Background(index - 1), controls);
                        return vec![];
                    }
                    Direction::Up => {
                        self.focus(
                            if index < 3 {
                                Target::UploadWallpaper
                            } else {
                                Target::Background(index - 3)
                            },
                            controls,
                        );
                        return vec![];
                    }
                    Direction::Down => {
                        if (index / 3 + 1) * 3 < self.background_count() {
                            self.focus(
                                Target::Background((index + 3).min(self.background_count() - 1)),
                                controls,
                            );
                        }
                        return vec![];
                    }
                    _ => {}
                }
            }
            if self.focus == Target::UploadWallpaper {
                match direction {
                    Direction::Down => self.focus(Target::Background(0), controls),
                    Direction::Up => {}
                    Direction::Left => self.focus(Target::Sidebar(1), controls),
                    Direction::Right => {}
                }
                return vec![];
            }
            if self.page == Page::Settings
                && matches!(direction, Direction::Left)
                && !matches!(self.focus, Target::Sidebar(_))
            {
                if let Target::Background(index) = self.focus {
                    if index % 3 != 0 { /* left follows gallery order */
                    } else {
                        self.focus(Target::Sidebar(self.section), controls);
                        return vec![];
                    }
                } else {
                    self.focus(Target::Sidebar(self.section), controls);
                    return vec![];
                }
            }
        }
        let Some(current) = controls.iter().find(|c| c.target == self.focus) else {
            self.focus(self.first_content(), controls);
            return vec![];
        };
        let (x, y) = current.rect.center();
        let next = controls
            .iter()
            .filter(|candidate| candidate.target != self.focus)
            .filter_map(|candidate| {
                let (cx, cy) = candidate.rect.center();
                let (primary, cross, overlap) = match direction {
                    Direction::Up => (
                        y - cy,
                        (x - cx).abs(),
                        (current.rect.w + candidate.rect.w) / 2.,
                    ),
                    Direction::Down => (
                        cy - y,
                        (x - cx).abs(),
                        (current.rect.w + candidate.rect.w) / 2.,
                    ),
                    Direction::Left => (
                        x - cx,
                        (y - cy).abs(),
                        (current.rect.h + candidate.rect.h) / 2.,
                    ),
                    Direction::Right => (
                        cx - x,
                        (y - cy).abs(),
                        (current.rect.h + candidate.rect.h) / 2.,
                    ),
                };
                (primary > 1.).then_some((
                    candidate.target.clone(),
                    primary + cross * 2. + if cross >= overlap { 10000. } else { 0. },
                ))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(target, _)| target);
        if let Some(next) = next {
            self.focus(next, controls);
        }
        vec![]
    }

    /// Remote routing on the main screen, by index rather than geometry so offscreen rows
    /// and the top shelf cannot steal it: the grid of apps has six columns;
    /// input cards, when shown, are rows of six above it; the shelf's weather button is
    /// above both. `None` leaves other targets to the generic routing; `Some(None)` consumes
    /// the press without moving.
    fn home_direction(&self, direction: Direction) -> Option<Option<Target>> {
        let grid = self.grid();
        // Inputs live in the top bar now, not in a block of cards above the grid.
        let inputs: Vec<Target> = Vec::new();
        let weather = (self.preferences.show_weather
            && self.preferences.weather_location.is_none())
        .then_some(Target::WeatherConfigure);
        // The top bar's icons, right-aligned on the clock's line.
        let bar = self.top_bar();
        // The bar item above a card column: the right-hand columns line up with the icons.
        let above = |column: usize| {
            let from_right = APPS_COLUMNS - column;
            bar.get(bar.len().saturating_sub(from_right)).cloned()
        };
        // Up from the top cards: the shelf's weather button above the left-hand columns, the
        // icons above the right-hand ones (or everywhere without a weather button).
        let up_from = |column: usize| {
            if column < APPS_COLUMNS / 2 && weather.is_some() {
                weather.clone()
            } else {
                above(column)
            }
        };
        // Moving within one block of six-column rows; leaving it by its top or bottom edge.
        let step = |items: &[Target], index: usize| -> Result<Option<Target>, bool> {
            let column = index % APPS_COLUMNS;
            match direction {
                Direction::Left if column > 0 => Ok(Some(items[index - 1].clone())),
                Direction::Right if column + 1 < APPS_COLUMNS && index + 1 < items.len() => {
                    Ok(Some(items[index + 1].clone()))
                }
                Direction::Left | Direction::Right => Ok(None),
                Direction::Up if index >= APPS_COLUMNS => {
                    Ok(Some(items[index - APPS_COLUMNS].clone()))
                }
                Direction::Up => Err(true),
                Direction::Down if (index / APPS_COLUMNS + 1) * APPS_COLUMNS < items.len() => Ok(
                    Some(items[(index + APPS_COLUMNS).min(items.len() - 1)].clone()),
                ),
                Direction::Down => Err(false),
            }
        };
        // Entering a block from above or below keeps the column where possible.
        let enter = |items: &[Target], column: usize, from_above: bool| {
            if items.is_empty() {
                return None;
            }
            let row = if from_above {
                0
            } else {
                (items.len() - 1) / APPS_COLUMNS
            };
            items
                .get((row * APPS_COLUMNS + column).min(items.len() - 1))
                .cloned()
        };
        // Down from the shelf or the bar enters the first block of cards.
        let below = |column: usize| {
            if inputs.is_empty() {
                enter(&grid, column, true)
                    .or_else(|| self.inventory_failed_empty().then_some(Target::Retry))
            } else {
                enter(&inputs, column, true)
            }
        };
        if let Some(index) = grid.iter().position(|item| *item == self.focus) {
            return Some(match step(&grid, index) {
                Ok(next) => next,
                Err(true) if !inputs.is_empty() => enter(&inputs, index % APPS_COLUMNS, false),
                Err(true) => up_from(index % APPS_COLUMNS),
                Err(false) => self.inventory_failed_empty().then_some(Target::Retry),
            });
        }
        if let Some(index) = inputs.iter().position(|item| *item == self.focus) {
            return Some(match step(&inputs, index) {
                Ok(next) => next,
                Err(true) => up_from(index % APPS_COLUMNS),
                Err(false) => enter(&grid, index % APPS_COLUMNS, true),
            });
        }
        if self.focus == Target::WeatherConfigure {
            return Some(match direction {
                Direction::Right => bar.first().cloned(),
                Direction::Down => below(0),
                _ => None,
            });
        }
        if let Some(index) = bar.iter().position(|item| *item == self.focus) {
            let column = (APPS_COLUMNS + index).saturating_sub(bar.len());
            return Some(match direction {
                Direction::Left if index > 0 => Some(bar[index - 1].clone()),
                Direction::Left => weather.clone(),
                Direction::Right => bar.get(index + 1).cloned(),
                Direction::Down => below(column),
                Direction::Up => None,
            });
        }
        None
    }

    /// The main screen's top bar, left to right: the visible inputs when Show widget is on, then
    /// the LG app store (when installed) and Settings.
    pub fn top_bar(&self) -> Vec<Target> {
        let store = self
            .inventory
            .apps
            .iter()
            .any(|app| app.id == STORE_ID)
            .then_some(Target::StoreIcon);
        self.home_inputs()
            .iter()
            .map(|input| Target::Input(input.id.clone()))
            .chain(store)
            .chain([Target::SettingsIcon])
            .collect()
    }

    /// Long OK on an input glyph: its existing settings (show mode and name) in
    /// Settings › Widgets › Inputs.
    fn input_options(&mut self, id: String) {
        self.open(Page::Settings);
        self.section = WIDGETS_SECTION;
        self.subpage = Some(Subpage::Inputs);
        self.focus = Target::InputMode(id);
        self.activate();
    }

    /// Builds `build` from the options overlay a long press on the focused item would open,
    /// then leaves everything as it was: the menu's labels can be rasterised while OK is still
    /// held, so the menu is complete in the frame the hold fires. `None` when a long press opens
    /// no menu here (input settings, or an overlay already open).
    pub fn with_hold_preview<T>(&mut self, build: impl FnOnce(&State) -> T) -> Option<T> {
        if self.overlay.is_some() || self.moving.is_some() {
            return None;
        }
        let (focus, dirty) = (self.focus.clone(), self.dirty);
        self.options();
        let result = self.overlay.is_some().then(|| build(self));
        self.overlay = None;
        self.focus = focus;
        self.dirty = dirty;
        result
    }

    pub fn options(&mut self) {
        if self.overlay.is_some() || self.moving.is_some() {
            return;
        }
        if let Target::Background(index) = self.focus {
            if let Some(id) = self.background_id(index).map(str::to_owned) {
                self.overlay = Some(Overlay::BackgroundOptions {
                    id,
                    restore: self.focus.clone(),
                });
                self.focus = Target::UseBackground;
                self.dirty = true;
            }
        } else if let Target::Tile(id) = &self.focus {
            if self.preferences.custom_tile(id).is_none() {
                return;
            }
            self.overlay = Some(Overlay::Options {
                id: id.clone(),
                restore: self.focus.clone(),
            });
            self.focus = Target::TileTest(id.clone());
            self.dirty = true;
        } else if let Target::App(id) = &self.focus {
            self.overlay = Some(Overlay::Options {
                id: id.clone(),
                restore: self.focus.clone(),
            });
            // Move is the first item, so a long press then OK starts moving the app.
            self.focus = Target::Option(0);
            self.dirty = true;
        }
    }

    pub fn uninstall_response(&mut self, id: String, result: Result<(), String>) -> Vec<Effect> {
        if self.uninstalling.as_ref() != Some(&id) {
            return vec![];
        }
        self.uninstalling = None;
        self.dirty = true;
        if let Err(error) = result {
            eprintln!("Luma uninstall: {error}");
            self.notify(self.preferences.language.t("deleteAppError"));
            if matches!(&self.overlay, Some(Overlay::Delete { id: current, .. } | Overlay::Options { id: current, .. }) if current == &id)
            {
                if let Some(Overlay::Delete { restore, .. } | Overlay::Options { restore, .. }) =
                    self.overlay.take()
                {
                    self.focus = restore;
                }
                self.pressed = None;
                self.reconcile_focus();
            }
            // Even an uncertain/failed mutation may have changed TV inventory.
            // Refresh is read-only, and also clears a stale card after late removal.
            return vec![Effect::Refresh];
        }
        let old_ids: Vec<_> = self.apps().iter().map(|app| app.id.clone()).collect();
        let index = old_ids.iter().position(|item| item == &id).unwrap_or(0);
        if let Some(order) = &mut self.preferences.app_order {
            order.retain(|item| item != &id);
        }
        if let Some(favorites) = &mut self.preferences.favorites {
            favorites.retain(|item| item != &id);
        }
        self.preferences.hidden.retain(|item| item != &id);
        self.inventory.apps.retain(|app| app.id != id);
        if matches!(&self.overlay, Some(Overlay::Delete { id: current, .. } | Overlay::Options { id: current, .. }) if current == &id)
        {
            self.overlay = None;
            self.focus = Target::App(id.clone());
        }
        if self.focus == Target::App(id) {
            self.focus = self
                .apps()
                .get(index.min(self.apps().len().saturating_sub(1)))
                .map(|app| Target::App(app.id.clone()))
                .unwrap_or_else(|| self.first_content());
        }
        self.reconcile_focus();
        self.notify(self.preferences.language.t("appDeleted"));
        vec![Effect::Save, Effect::Refresh]
    }

    /// Holding OK (or the Menu key) on any app opens its options, which start with Move; on
    /// an input glyph it opens that input's settings.
    pub fn hold(&mut self) {
        if self.page == Page::Home
            && self.overlay.is_none()
            && let Target::Input(id) = &self.focus
        {
            let id = id.clone();
            self.input_options(id);
            return;
        }
        self.options();
    }

    pub fn text(&mut self, text: &str) {
        if self.text_input_active() {
            if self.keyboard_field == TextField::Location {
                let mut query = self.weather.query.clone();
                query.extend(
                    text.chars()
                        .filter(|c| !c.is_control())
                        .take(120usize.saturating_sub(query.chars().count())),
                );
                self.weather.edit(query);
                self.dirty = true;
                return;
            }
            if self.keyboard_field == TextField::Name {
                let name = &mut self.greeting_name_draft;
                name.extend(
                    text.chars()
                        .filter(|c| !c.is_control())
                        .take(40usize.saturating_sub(name.chars().count())),
                );
                self.dirty = true;
                return;
            }
            if self.keyboard_field == TextField::InputName {
                let length = self.input_name.chars().count();
                self.input_name.extend(
                    text.chars()
                        .filter(|c| !c.is_control())
                        .take(INPUT_NAME_LIMIT.saturating_sub(length)),
                );
                self.dirty = true;
            }
        }
    }

    pub fn delete(&mut self) {
        if self.text_input_active() {
            if self.keyboard_field == TextField::Location {
                let mut query = self.weather.query.clone();
                query.pop();
                self.weather.edit(query);
            } else if self.keyboard_field == TextField::Name {
                self.greeting_name_draft.pop();
            } else if self.keyboard_field == TextField::InputName {
                self.input_name.pop();
            }
            self.dirty = true;
        }
    }

    pub fn text_input_active(&self) -> bool {
        matches!(
            self.overlay,
            Some(Overlay::Keyboard | Overlay::SystemKeyboard)
        )
    }

    pub fn finish_text_input(&mut self) -> Vec<Effect> {
        if self.text_input_active() {
            self.overlay = None;
            self.focus = self.keyboard_return();
            self.pointer = false;
            self.dirty = true;
            return self.commit_text_input();
        }
        vec![]
    }

    fn commit_text_input(&mut self) -> Vec<Effect> {
        if self.keyboard_field == TextField::Name {
            let name = std::mem::take(&mut self.greeting_name_draft);
            let name = name.trim();
            // Empty dismissal preserves the saved value; Clear name is explicit.
            if !name.is_empty() && self.preferences.greeting_name != name {
                self.preferences.greeting_name = name.into();
                return vec![Effect::Save];
            }
            return vec![];
        }
        self.commit_input_name()
    }

    /// Closing the keyboard by any route keeps a typed input name. An empty
    /// field leaves the saved name unchanged; Clear name removes it explicitly.
    fn commit_input_name(&mut self) -> Vec<Effect> {
        let name = std::mem::take(&mut self.input_name);
        if self.keyboard_field != TextField::InputName || clean_input_name(&name).is_empty() {
            return vec![];
        }
        let Some(Subpage::Input(id)) = self.subpage.clone() else {
            return vec![];
        };
        if self.preferences.set_input_name(&id, &name) {
            vec![Effect::Save]
        } else {
            vec![]
        }
    }

    /// Applies the tile list edited on a phone or computer. New tiles join
    /// the end of the app order and removed ones leave the order and hidden apps.
    pub fn replace_tiles(&mut self, tiles: Vec<CustomTile>) -> Vec<Effect> {
        let removed: Vec<_> = self
            .preferences
            .custom_tiles
            .iter()
            .filter(|old| !tiles.iter().any(|tile| tile.id == old.id))
            .map(|old| old.id.clone())
            .collect();
        for id in &removed {
            self.preferences.remove_tile(id);
        }
        let added: Vec<_> = tiles
            .iter()
            .filter(|tile| self.preferences.custom_tile(&tile.id).is_none())
            .map(|tile| tile.id.clone())
            .collect();
        self.preferences.custom_tiles = tiles;
        self.preferences.sanitize();
        self.sync_custom_tiles();
        if let Target::Tile(id) | Target::TileRemove(id) | Target::TileTest(id) = &self.focus
            && self.preferences.custom_tile(id).is_none()
        {
            if matches!(
                &self.overlay,
                Some(Overlay::Options {
                    restore: Target::Tile(_),
                    ..
                })
            ) {
                self.overlay = None;
            }
            self.focus = Target::TileEditor;
        }
        if !added.is_empty() {
            let mut order = self.preferences.ordered_ids(&self.inventory.apps);
            for id in added {
                if self.preferences.custom_tile(&id).is_some() && !order.contains(&id) {
                    order.push(id);
                }
            }
            self.preferences.app_order = Some(order);
        }
        vec![Effect::Save]
    }

    fn send_tile(&mut self, tile: CustomTile) -> Vec<Effect> {
        self.notify(
            self.preferences
                .language
                .format("tileSending", &[("tile", &tile.title)]),
        );
        vec![Effect::FireWebhook(tile)]
    }

    pub fn delete_surrounding_text(&mut self, offset: i32, length: i32) {
        if !self.text_input_active() || length <= 0 {
            return;
        }
        // SDL-webOS forwards the native text model's UTF-8 byte offsets,
        // relative to the insertion cursor (at the end of our query).
        let mut query = self.editing_text().to_owned();
        let mut start =
            (query.len() as i64 + i64::from(offset)).clamp(0, query.len() as i64) as usize;
        let mut end = start.saturating_add(length as usize).min(query.len());
        while !query.is_char_boundary(start) {
            start -= 1;
        }
        while !query.is_char_boundary(end) {
            end += 1;
        }
        query.replace_range(start..end, "");
        if self.keyboard_field == TextField::Location {
            self.weather.edit(query);
        } else if self.keyboard_field == TextField::Name {
            self.greeting_name_draft = query;
        } else if self.keyboard_field == TextField::InputName {
            self.input_name = query;
        }
        self.dirty = true;
    }

    pub fn editing_text(&self) -> &str {
        match self.keyboard_field {
            TextField::Location => &self.weather.query,
            TextField::Name => &self.greeting_name_draft,
            TextField::InputName => &self.input_name,
        }
    }
    pub fn open_keyboard(&mut self, field: TextField) {
        self.keyboard_field = field;
        // Suggestions follow what is typed now, not text already there when it opens.
        self.suggested_query = self.weather.query.clone();
        self.suggest_at = None;
        if field == TextField::Name {
            // SDL-webOS cannot pre-fill the system keyboard's text model.
            self.greeting_name_draft.clear();
        }
        if field == TextField::InputName {
            // The TV keyboard starts empty, so a new name replaces the old one.
            self.input_name.clear();
        }
        // The weather location uses Luma's own tvOS-style linear keyboard everywhere, so its
        // suggestions can sit beneath the keys; other fields use the TV's system keyboard.
        if field != TextField::Location
            && cfg!(all(
                feature = "webos",
                target_os = "linux",
                target_arch = "arm"
            ))
        {
            self.overlay = Some(Overlay::SystemKeyboard);
            self.focus = match field {
                TextField::Location => Target::WeatherQuery,
                TextField::Name => Target::GreetingName,
                TextField::InputName => Target::InputRename,
            };
            self.dirty = true;
            return;
        }
        self.overlay = Some(Overlay::Keyboard);
        self.focus = Target::Key(0);
        self.dirty = true;
    }
    fn open_location(&mut self) {
        self.weather.edit(String::new());
        self.subpage = Some(Subpage::WeatherLocation);
        self.focus = Target::WeatherQuery;
        self.scroll = 0.;
        self.dirty = true;
    }
    fn keyboard_return(&self) -> Target {
        match self.keyboard_field {
            TextField::Location => Target::WeatherSearch,
            TextField::Name => Target::GreetingName,
            TextField::InputName => Target::InputRename,
        }
    }

    pub fn activate(&mut self) -> Vec<Effect> {
        if self.recents_open() {
            use crate::recents::Command;
            return match self.focus.clone() {
                Target::Recent(id) => self.recent_request(Command::Launch(id)),
                Target::RecentRetry => self.recent_request(Command::List),
                _ => vec![],
            };
        }
        if self.moving.take().is_some() {
            // Order was saved at every move. A fresh OK release finishes the
            // mode and keeps the selected favorite, rather than launching it.
            self.dirty = true;
            return vec![];
        }
        self.dirty = true;
        if let Target::App(id) = &self.focus
            && let Some(tile) = self.preferences.custom_tile(id).cloned()
        {
            return self.send_tile(tile);
        }
        match self.focus.clone() {
            Target::SettingsIcon => self.open(Page::Settings),
            Target::StoreIcon if !self.launching => {
                return vec![Effect::Launch(STORE_ID.into())];
            }
            Target::Retry => return vec![Effect::Reload],
            Target::DismissNotice => self.notice = None,
            Target::CloseDialog => return self.back(),
            Target::UploadWallpaper => {
                self.wallpaper_upload = None;
                self.overlay = Some(Overlay::WallpaperUpload);
                self.focus = Target::CloseDialog;
                self.pressed = None;
            }
            Target::TileEditor => {
                self.tile_editor = None;
                self.overlay = Some(Overlay::TileEditor);
                self.focus = Target::CloseDialog;
                self.pressed = None;
            }
            Target::App(id) | Target::Input(id) if !self.launching => {
                return vec![Effect::Launch(id)];
            }
            Target::OpenApp if !self.launching => {
                if let Some(Overlay::Options { id, .. }) = &self.overlay
                    && self.inventory.apps.iter().any(|app| &app.id == id)
                {
                    let id = id.clone();
                    self.back();
                    if let Some(tile) = self.preferences.custom_tile(&id).cloned() {
                        return self.send_tile(tile);
                    }
                    return vec![Effect::Launch(id)];
                }
            }
            Target::WeatherConfigure => {
                self.open(Page::Settings);
                self.section = WIDGETS_SECTION;
                self.open_location();
                self.open_keyboard(TextField::Location);
            }
            Target::WeatherToggle => {
                self.preferences.show_weather = !self.preferences.show_weather;
                self.weather.invalidate_forecast();
                return vec![Effect::Save];
            }
            Target::WeatherLocation => self.open_location(),
            Target::WeatherTemperature => {
                self.subpage = Some(Subpage::Temperature);
                self.focus = Target::Choice(usize::from(
                    self.preferences.temperature_unit == Unit::Fahrenheit,
                ));
            }
            Target::WeatherQuery => self.open_keyboard(TextField::Location),
            Target::InputsToggle => {
                self.preferences.show_inputs = !self.preferences.show_inputs;
                return vec![Effect::Save];
            }
            Target::GreetingToggle => {
                self.preferences.show_greeting = !self.preferences.show_greeting;
                return vec![Effect::Save];
            }
            Target::DateToggle => {
                self.preferences.show_date = !self.preferences.show_date;
                return vec![Effect::Save];
            }
            Target::DateFormat => {
                self.subpage = Some(Subpage::DateFormat);
                self.focus = Target::Choice(self.preferences.date_format.index());
                self.scroll = 0.;
            }
            Target::GreetingName => self.open_keyboard(TextField::Name),
            Target::GreetingNameClear => {
                self.preferences.greeting_name.clear();
                self.focus = Target::GreetingName;
                return vec![Effect::Save];
            }
            Target::ClockFormat | Target::ClockTimeFormat => {
                let format = self.focus == Target::ClockFormat;
                self.subpage = Some(if format {
                    Subpage::ClockFormat
                } else {
                    Subpage::TimeFormat
                });
                self.focus = Target::Choice(usize::from(if format {
                    self.preferences.uses_24_hour()
                } else {
                    self.preferences.show_seconds
                }));
                self.scroll = 0.;
            }
            Target::InputRename => self.open_keyboard(TextField::InputName),
            Target::Widget(index) => {
                let (subpage, focus) = match index {
                    0 => (Subpage::Greeting, Target::GreetingToggle),
                    1 => (Subpage::Weather, Target::WeatherToggle),
                    2 => (Subpage::Shortcuts, Target::TileEditor),
                    4 => (Subpage::Date, Target::DateToggle),
                    // Like the other widgets, Inputs opens on its Show widget toggle.
                    3 => (Subpage::Inputs, Target::InputsToggle),
                    _ => return vec![],
                };
                self.section = WIDGETS_SECTION;
                self.subpage = Some(subpage);
                self.focus = focus;
                self.scroll = 0.;
            }
            Target::InstallUpdate => {
                if !self.update_busy()
                    && let Some(version) = &self.available_update
                {
                    let version = version.clone();
                    self.update = crate::update::InstallStatus::Downloading(0);
                    return vec![Effect::UpdateDownload(version)];
                }
            }
            Target::CheckUpdates => {
                if !self.update_busy() {
                    self.update = crate::update::InstallStatus::Idle;
                    self.update_status = crate::update::Status::Checking;
                    self.update_check_requested = true;
                    self.dirty = true;
                }
            }
            Target::Tile(_) => self.options(),
            Target::TileTest(id) => {
                if matches!(&self.overlay, Some(Overlay::Options { id: current, restore: Target::Tile(_), .. }) if current == &id)
                    && let Some(tile) = self.preferences.custom_tile(&id).cloned()
                {
                    return self.send_tile(tile);
                }
            }
            Target::TileRemove(id) => {
                if matches!(&self.overlay, Some(Overlay::Options { id: current, restore: Target::Tile(_), .. }) if current == &id)
                    && let Some(tile) = self.preferences.remove_tile(&id)
                {
                    self.overlay = None;
                    self.notify(
                        self.preferences
                            .language
                            .format("tileRemoved", &[("tile", &tile.title)]),
                    );
                    self.subpage = Some(Subpage::Shortcuts);
                    self.scroll = 0.;
                    self.sync_custom_tiles();
                    self.focus = Target::TileEditor;
                    return vec![Effect::Save];
                }
            }
            Target::InputNameReset => {
                if let Some(Subpage::Input(id)) = &self.subpage
                    && self.preferences.input_names.remove(id).is_some()
                {
                    self.focus = Target::InputRename;
                    return vec![Effect::Save];
                }
            }
            Target::WeatherSearch => {
                return self
                    .weather
                    .search(self.preferences.language)
                    .map(|command| vec![Effect::Weather(command)])
                    .unwrap_or_default();
            }
            Target::WeatherClear => {
                self.preferences.weather_location = None;
                self.weather.remove_location();
                self.focus = Target::WeatherSearch;
                return vec![Effect::Save, Effect::Weather(weather::Command::ClearCache)];
            }
            Target::WeatherResult(index) => {
                if let Some(place) = self.weather.results.get(index).cloned() {
                    // A suggestion picked from the keyboard also closes it.
                    if matches!(self.overlay, Some(Overlay::Keyboard)) {
                        self.overlay = None;
                    }
                    self.preferences.weather_location = Some(place);
                    self.weather.invalidate_forecast();
                    self.weather.cancel_search();
                    self.focus = Target::WeatherSearch;
                    return vec![Effect::Save];
                }
            }
            Target::Sidebar(index) => {
                self.section = index;
                self.scroll = 0.;
                self.subpage = None;
                self.focus = self.section_entry();
            }
            Target::Setting(index) => {
                match index {
                    // One grid now, so app names are a single toggle (focused card only).
                    0 => {
                        self.preferences.show_app_names = !self.preferences.show_app_names;
                        return vec![Effect::Save];
                    }
                    1 => {
                        self.subpage = Some(Subpage::Clock);
                        self.scroll = 0.;
                        self.focus = Target::ClockFormat;
                        return vec![];
                    }
                    12 => {
                        self.subpage = Some(Subpage::Screensaver);
                        self.scroll = 0.;
                        self.focus = Target::Choice(
                            SCREENSAVER_MINUTES
                                .iter()
                                .position(|m| *m == self.preferences.screensaver_minutes)
                                .unwrap_or(3),
                        );
                        return vec![];
                    }
                    14 => {
                        self.subpage = Some(Subpage::Appearance);
                        self.scroll = 0.;
                        self.focus = Target::Choice(self.preferences.appearance.index());
                        return vec![];
                    }
                    11 => {
                        self.subpage = Some(Subpage::HiddenApps);
                        self.scroll = 0.;
                        self.focus = Target::Choice(0);
                        return vec![];
                    }
                    4 => {
                        self.subpage = Some(Subpage::Language);
                        self.focus = Target::Choice(self.preferences.language.index());
                        return vec![];
                    }
                    5 => return vec![Effect::Quit],
                    // Temporary focus size choice: OK cycles 6% to 14% and applies at once.
                    16 => {
                        self.preferences.focus_grow = self.preferences.focus_grow.next();
                        self.dirty = true;
                        return vec![Effect::Save];
                    }
                    _ => return vec![Effect::Reload],
                }
            }
            Target::Background(_) => {
                self.options();
            }
            Target::UseBackground => {
                let Some(Overlay::BackgroundOptions { id, .. }) = self.overlay.clone() else {
                    return vec![];
                };
                if self.deleting_wallpaper.as_deref() == Some(&id) {
                    return vec![];
                }
                if !self.background_ids().any(|known| known == id) {
                    return self.back();
                }
                if self.preferences.background != id {
                    self.previous_background =
                        Some((self.preferences.background.clone(), Instant::now()));
                    self.background_fade = BACKGROUND_FADE;
                }
                self.preferences.splash_background = Some(if crate::wallpapers::valid_id(&id) {
                    self.preferences.splash_background().to_owned()
                } else {
                    id.clone()
                });
                self.preferences.background = id;
                self.back();
                return vec![Effect::Save];
            }
            Target::DeleteBackground => {
                if let Some(Overlay::BackgroundOptions { id, restore }) = self.overlay.clone()
                    && self.can_delete_background(&id)
                    && self.deleting_wallpaper.is_none()
                {
                    self.overlay = Some(Overlay::BackgroundDelete { id, restore });
                    self.focus = Target::CancelDelete;
                    self.pointer = false;
                    self.pressed = None;
                }
            }
            Target::InputMode(id) => {
                let mode = self
                    .preferences
                    .input_modes
                    .get(&id)
                    .copied()
                    .unwrap_or_default();
                self.subpage = Some(Subpage::Input(id));
                self.focus = Target::Choice(match mode {
                    InputMode::On => 0,
                    InputMode::Off => 1,
                    InputMode::Auto => 2,
                });
            }
            Target::Choice(index) => {
                match self.subpage.clone() {
                    Some(Subpage::ClockFormat) => self.preferences.set_24_hour(index == 1),
                    Some(Subpage::TimeFormat) => self.preferences.show_seconds = index == 1,
                    Some(Subpage::DateFormat) => {
                        self.preferences.date_format = crate::model::DateFormat::ALL[index.min(3)];
                    }
                    Some(Subpage::Input(id)) => {
                        self.preferences.input_modes.insert(
                            id,
                            [InputMode::On, InputMode::Off, InputMode::Auto][index.min(2)],
                        );
                    }
                    Some(Subpage::Temperature) => {
                        self.preferences.temperature_unit = if index == 1 {
                            Unit::Fahrenheit
                        } else {
                            Unit::Celsius
                        }
                    }
                    Some(Subpage::Language) => {
                        self.preferences.language =
                            Language::ALL[index.min(Language::ALL.len() - 1)];
                        self.weather.cancel_search();
                    }
                    Some(Subpage::Appearance) => {
                        let modes = crate::model::Appearance::ALL;
                        self.preferences.appearance = modes[index.min(modes.len() - 1)];
                    }
                    Some(Subpage::Screensaver) => {
                        self.preferences.screensaver_minutes =
                            SCREENSAVER_MINUTES[index.min(SCREENSAVER_MINUTES.len() - 1)];
                    }
                    Some(Subpage::HiddenApps) => {
                        // Showing an app again keeps the list open on the next hidden app.
                        let Some((id, title)) = self
                            .hidden_apps()
                            .get(index)
                            .map(|app| (app.id.clone(), app.title.clone()))
                        else {
                            return vec![];
                        };
                        self.preferences.hidden.retain(|hidden| hidden != &id);
                        let remaining = self.hidden_apps().len();
                        self.focus = Target::Choice(index.min(remaining.saturating_sub(1)));
                        self.notify(
                            self.preferences
                                .language
                                .format("appRestored", &[("app", &title)]),
                        );
                        return vec![Effect::Save];
                    }
                    Some(
                        Subpage::WeatherLocation
                        | Subpage::Greeting
                        | Subpage::Date
                        | Subpage::Clock
                        | Subpage::Weather
                        | Subpage::Shortcuts
                        | Subpage::Inputs,
                    ) => {
                        return vec![];
                    }
                    None => return vec![],
                }
                self.close_subpage();
                return vec![Effect::Save];
            }
            Target::Option(index) => {
                if index == 2 {
                    if let Some(Overlay::Options { id, restore }) = &self.overlay
                        && crate::platform::can_uninstall(id)
                        && self.uninstalling.is_none()
                        && self.inventory.apps.iter().any(|app| &app.id == id)
                    {
                        self.overlay = Some(Overlay::Delete {
                            id: id.clone(),
                            restore: restore.clone(),
                        });
                        self.focus = Target::CancelDelete;
                        self.pointer = false;
                        self.pressed = None;
                    }
                    return vec![];
                }
                let Some(Overlay::Options { id, restore }) = self.overlay.take() else {
                    return vec![];
                };
                let old_ids: Vec<_> = self.apps().iter().map(|a| a.id.clone()).collect();
                let old_index = old_ids.iter().position(|item| item == &id).unwrap_or(0);
                let title = self
                    .inventory
                    .apps
                    .iter()
                    .find(|app| app.id == id)
                    .map_or(id.clone(), |app| app.title.clone());
                if index == 0 {
                    // Move: the card enters move mode. The d-pad moves it one slot at a time
                    // (or the pointer drags it); OK or Back keeps the new order.
                    self.focus = Target::App(id.clone());
                    self.moving = Some(id);
                    self.notice = None;
                    self.pressed = None;
                    self.dirty = true;
                    return vec![];
                } else if index == 1 {
                    if self.preferences.hidden.contains(&id) {
                        self.preferences.hidden.retain(|item| item != &id);
                        self.notify(
                            self.preferences
                                .language
                                .format("appRestored", &[("app", &title)]),
                        );
                    } else {
                        self.preferences.hidden.push(id.clone());
                        self.notify(
                            self.preferences
                                .language
                                .format("appHidden", &[("app", &title)]),
                        );
                    }
                }
                self.focus = restore;
                self.pointer = false;
                if let Target::App(id) = &self.focus {
                    let apps = self.apps();
                    if !apps.iter().any(|app| &app.id == id) {
                        self.focus = apps
                            .get(old_index.min(apps.len().saturating_sub(1)))
                            .map(|app| Target::App(app.id.clone()))
                            .unwrap_or_else(|| self.first_content());
                    }
                }
                return if index < 2 {
                    vec![Effect::Save]
                } else {
                    vec![]
                };
            }
            Target::CancelDelete => return self.back(),
            Target::ConfirmDelete => {
                if let Some(Overlay::BackgroundDelete { id, .. }) = self.overlay.clone() {
                    if self.can_delete_background(&id) && self.deleting_wallpaper.is_none() {
                        if crate::wallpapers::valid_id(&id) {
                            self.deleting_wallpaper = Some(id.clone());
                            self.focus = Target::CancelDelete;
                            return vec![Effect::DeleteWallpaper(id)];
                        }
                        // Packaged assets remain intact; persist removal from the gallery.
                        self.finish_background_delete(&id);
                        return vec![Effect::Save];
                    }
                    return vec![];
                }
                if let Some(Overlay::Delete { id, .. }) = &self.overlay
                    && self.uninstalling.is_none()
                    && crate::platform::can_uninstall(id)
                    && self.inventory.apps.iter().any(|app| &app.id == id)
                {
                    self.uninstalling = Some(id.clone());
                    self.focus = Target::CancelDelete;
                    return vec![Effect::Uninstall(id.clone())];
                }
            }
            Target::Key(index) => match KEYS[index] {
                "Space" => self.text(" "),
                "Delete" => self.delete(),
                "Clear" => match self.keyboard_field {
                    TextField::Location => self.weather.edit(String::new()),
                    TextField::Name => self.greeting_name_draft.clear(),
                    TextField::InputName => self.input_name.clear(),
                },
                "Done" => {
                    self.overlay = None;
                    self.focus = self.keyboard_return();
                    return self.commit_text_input();
                }
                key => self.text(key),
            },
            _ => {}
        }
        vec![]
    }

    pub fn back(&mut self) -> Vec<Effect> {
        self.pointer = false;
        self.dirty = true;
        if self.moving.take().is_some() {
            self.drag = None;
            return vec![];
        }
        if let Some(overlay) = self.overlay.take() {
            if let Overlay::BackgroundDelete { id, restore } = overlay {
                self.overlay = Some(Overlay::BackgroundOptions { id, restore });
                self.focus = Target::DeleteBackground;
                self.pressed = None;
                return vec![];
            }
            if let Overlay::BackgroundOptions { id, .. } = overlay {
                self.focus = self.background_target(&id);
                self.pressed = None;
                return vec![];
            }
            if matches!(overlay, Overlay::WallpaperUpload) {
                self.wallpaper_upload = None;
                self.focus = Target::UploadWallpaper;
                self.pressed = None;
                return vec![];
            }
            if matches!(overlay, Overlay::TileEditor) {
                self.tile_editor = None;
                self.focus = Target::TileEditor;
                self.pressed = None;
                return vec![];
            }
            if let Overlay::Delete { id, restore } = overlay {
                self.overlay = Some(Overlay::Options { id, restore });
                self.focus = Target::Option(2);
                self.pressed = None;
                return vec![];
            }
            if matches!(overlay, Overlay::Recents { .. }) {
                // Gone in the very frame it closes (no fade).
                self.recents.updated = Instant::now();
                self.recents.visibility = 0.;
                self.recents.closing_focus = None;
                self.pressed = None;
            }
            if let Overlay::Options { restore, .. } | Overlay::Recents { restore } = overlay {
                self.focus = restore;
                self.recents.pending = None;
                self.reconcile_focus();
                return vec![];
            }
            self.focus = self.keyboard_return();
            return self.commit_text_input();
        }
        if self.page == Page::Settings && self.subpage.is_some() {
            self.close_subpage();
            return vec![];
        }
        if self.page == Page::Settings {
            if matches!(self.focus, Target::Sidebar(_)) {
                // Leaving Settings returns to its card in the grid.
                self.open(Page::Home);
                self.focus = Target::SettingsIcon;
            } else {
                self.focus = Target::Sidebar(self.section);
            }
            self.scroll = 0.;
            return vec![];
        }
        self.open_recents()
    }
}
