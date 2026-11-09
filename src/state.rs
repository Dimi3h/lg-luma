use crate::i18n::Language;
use crate::model::{
    App, BACKGROUNDS, CUSTOM_TILE_PREFIX, CustomTile, INPUT_NAME_LIMIT, InputMode, Inventory,
    Preferences, clean_input_name,
};
use crate::text_rendering::TextRendering;
use crate::weather::{self, Unit};
use serde::Serialize;
use std::time::{Duration, Instant};

pub const WIDGETS_SECTION: usize = 2;
pub const SYSTEM_SECTION: usize = 3;
pub const SECTION_COUNT: usize = 4;
pub const SETTINGS_BOTTOM_PADDING: f32 = 64.;
pub const SETTINGS_LIST_BOTTOM_PADDING: f32 = 48.;
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
pub const CYCLE_FADE: f32 = 1.6;
/// Crossfade of the top-shelf wash between apps, in seconds.
pub const WASH_FADE: f32 = 0.35;

/// A card of the main screen's grid (inputs, apps and the system cards).
fn is_grid_card(target: &Target) -> bool {
    matches!(
        target,
        Target::App(_) | Target::Input(_) | Target::SettingsCard
    )
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

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
    /// The grid's last card, which opens Settings.
    SettingsCard,
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
    WallpaperCycle,
    WallpaperGradient,
    UseBackground,
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
    TextRendering,
    /// Settings › General › Hidden apps: hidden apps as rows, OK shows one again.
    HiddenApps,
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
    /// The Magic Remote pointer's last logical position (for the focused card's tilt).
    pub pointer_at: Option<(f32, f32)>,
    pub pressed: Option<Target>,
    pub previous_background: Option<(String, Instant)>,
    pub background_fade: f32,
    pub wallpapers: Vec<crate::wallpapers::Wallpaper>,
    pub deleting_wallpaper: Option<String>,
    pub wallpaper_upload: Option<Result<crate::wallpapers::Session, String>>,
    pub tile_editor: Option<Result<crate::companion::Session, String>>,
    /// Luma update (Settings → System), driven by the update worker.
    pub update: crate::update::InstallStatus,
    pub panel_started: Option<Instant>,
    pub weather: weather::Weather,
    pub keyboard_field: TextField,
    pub greeting_name_draft: String,
    pub input_name: String,
    pub recents: crate::recents::Recents,
    /// Display-only easing of focus, scrolling and page switches.
    pub motion: crate::motion::Motion,
    /// The top-shelf wash: the app whose artwork shows behind the shelf, since when, and the
    /// one fading out (Widgets › App artwork).
    pub wash: Option<(String, Instant)>,
    pub wash_previous: Option<(String, Instant)>,
}

impl State {
    pub fn new(preferences: Preferences) -> Self {
        Self {
            page: Page::Home,
            focus: Target::SettingsCard,
            inventory: Inventory::default(),
            preferences,
            section: 0,
            subpage: None,
            scroll: 0.,
            overlay: None,
            moving: None,
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
            pointer_at: None,
            pressed: None,
            previous_background: None,
            background_fade: BACKGROUND_FADE,
            wallpapers: vec![],
            deleting_wallpaper: None,
            wallpaper_upload: None,
            tile_editor: None,
            update: crate::update::InstallStatus::default(),
            panel_started: None,
            weather: weather::Weather::default(),
            keyboard_field: TextField::Location,
            greeting_name_draft: String::new(),
            input_name: String::new(),
            recents: crate::recents::Recents::default(),
            motion: crate::motion::Motion::default(),
            wash: None,
            wash_previous: None,
        }
    }

    /// The app whose artwork washes the top shelf: the focused app in the first row.
    fn wash_target(&self) -> Option<String> {
        if !self.preferences.app_artwork || self.page != Page::Home || self.overlay.is_some() {
            return None;
        }
        match &self.focus {
            Target::App(id) if self.in_first_row(&self.focus) => Some(id.clone()),
            _ => None,
        }
    }

    /// Follows focus with the top-shelf wash, crossfading between apps (an instant swap under
    /// Reduced motion). Called before each scene is built.
    pub fn sync_wash(&mut self) {
        let target = self.wash_target();
        if self.wash.as_ref().map(|(id, _)| id) != target.as_ref() {
            let now = Instant::now();
            let previous = self.wash.take().map(|(id, _)| (id, now));
            self.wash_previous = previous.filter(|_| !self.preferences.reduced_motion);
            self.wash = target.map(|id| (id, now));
            self.dirty = true;
        }
        if self
            .wash_previous
            .as_ref()
            .is_some_and(|(_, since)| since.elapsed().as_secs_f32() >= WASH_FADE)
        {
            self.wash_previous = None;
            self.dirty = true;
        }
    }

    /// How far a wash shown or hidden at `since` has faded in (1 = complete).
    pub fn wash_progress(&self, since: Instant) -> f32 {
        if self.preferences.reduced_motion {
            1.
        } else {
            (since.elapsed().as_secs_f32() / WASH_FADE).min(1.)
        }
    }

    fn motion_inputs(&self) -> crate::motion::Inputs<'_> {
        crate::motion::Inputs {
            focus: &self.focus,
            page: self.page,
            section: self.section,
            subpage: &self.subpage,
            scroll: self.scroll,
            reduced: self.preferences.reduced_motion,
            pointer: self.pointer,
            tilt: self.tilt_enabled(),
            pointer_at: self.pointer_at,
        }
    }

    /// Focus tilt is on and motion is not reduced.
    pub fn tilt_enabled(&self) -> bool {
        self.preferences.focus_tilt && !self.preferences.reduced_motion
    }

    /// Notes where the focused card rests, from a composed scene, for pointer tilt.
    pub fn note_focus_rect(&mut self, controls: &[Control]) {
        let card = matches!(
            self.focus,
            Target::App(_) | Target::Input(_) | Target::SettingsCard
        );
        let rect = card
            .then(|| controls.iter().find(|control| control.target == self.focus))
            .flatten()
            .map(|control| control.rect);
        self.motion.set_focus_rect(rect);
    }

    /// Registers the current focus before a scene is composed (see `Motion::sync`).
    pub fn sync_motion(&mut self) {
        let mut motion = std::mem::take(&mut self.motion);
        motion.sync(&self.motion_inputs());
        self.motion = motion;
    }

    /// How far `target` is lifted by focus, 0 at rest and 1 fully focused.
    pub fn focus_level(&self, target: &Target) -> f32 {
        self.motion.focus_level(target, &self.focus)
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
        let mut motion = std::mem::take(&mut self.motion);
        self.dirty |= motion.advance(&self.motion_inputs(), Instant::now());
        self.motion = motion;
        let recents_open = self.recents_open();
        if recents_open || self.recents.visibility > 0. {
            self.dirty |= self.recents.animate(
                recents_open,
                self.preferences.reduced_motion,
                self.pointer,
                Instant::now(),
            );
        }
        if let Some(start) = self.panel_started {
            self.dirty = true;
            let duration = if self.overlay.is_some() { 120 } else { 150 };
            if self.preferences.reduced_motion || start.elapsed() >= Duration::from_millis(duration)
            {
                self.panel_started = None;
            }
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, start)| start.elapsed() < Duration::from_millis(240))
            && !self.preferences.reduced_motion
        {
            self.dirty = true;
        }
        if let Some((_, start)) = &self.previous_background {
            self.dirty = true;
            if self.preferences.reduced_motion
                || start.elapsed().as_secs_f32() >= self.background_fade
            {
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
            || self.panel_started.is_some()
            || self.motion.animating()
            || self.wash_previous.is_some()
            || self
                .wash
                .as_ref()
                .is_some_and(|(_, since)| self.wash_progress(*since) < 1.)
            || (!self.preferences.reduced_motion
                && self
                    .notice
                    .as_ref()
                    .is_some_and(|(_, start)| start.elapsed() < Duration::from_millis(240)))
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
        self.subpage = None;
        self.panel_started = None;
        if page == Page::Settings {
            self.section = 0;
        }
        self.focus = self.first_content();
        self.dirty = true;
    }

    /// The main grid's apps: pinned favourites in their saved order, then every other visible
    /// app by title. Hidden apps are listed in Settings › General › Hidden apps instead.
    pub fn apps(&self) -> Vec<&App> {
        let ids = self.preferences.favorites(&self.inventory.apps);
        let mut apps: Vec<_> = self
            .inventory
            .apps
            .iter()
            .filter(|app| !self.preferences.hidden.contains(&app.id))
            .collect();
        apps.sort_by(|a, b| {
            let rank = |app: &App| {
                ids.iter()
                    .position(|id| id == &app.id)
                    .unwrap_or(usize::MAX)
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        apps
    }

    /// Hidden apps that are still installed, by title (Settings › General › Hidden apps).
    pub fn hidden_apps(&self) -> Vec<&App> {
        let mut apps: Vec<_> = self
            .inventory
            .apps
            .iter()
            .filter(|app| self.preferences.hidden.contains(&app.id))
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
        grid.push(Target::SettingsCard);
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
        viewport.y + viewport.h
            - if self.page == Page::Settings {
                if self.subpage == Some(Subpage::Language)
                    || (self.section == 1 && self.subpage.is_none())
                {
                    SETTINGS_LIST_BOTTOM_PADDING
                } else {
                    SETTINGS_BOTTOM_PADDING
                }
            } else {
                // Room for the focused caption inside the 60 px bottom safe area.
                crate::view::CAPTION_SPACE + 60.
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

    pub fn set_inventory(&mut self, next: Inventory, warning: Option<String>) {
        self.inventory.apps = next.apps;
        self.sync_custom_tiles();
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
                .map_or(Target::SettingsCard, |app| Target::App(app.id.clone())),
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
            Target::Input(id) => self
                .home_inputs()
                .iter()
                .position(|input| &input.id == id)
                .is_some_and(|index| index < APPS_COLUMNS),
            Target::App(_) | Target::SettingsCard => self
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

    pub fn cycle_wallpaper(&mut self, now: u64) -> bool {
        let ids: Vec<_> = self
            .background_ids()
            .filter(|id| self.deleting_wallpaper.as_deref() != Some(*id))
            .map(str::to_owned)
            .collect();
        let next = crate::wallpaper_cycle::next(&ids, &self.preferences.background);
        self.preferences.wallpaper_changed = now;
        let Some(id) = next else {
            return false;
        };
        self.preferences.splash_background = Some(self.preferences.splash_background().to_owned());
        if self.preferences.reduced_motion {
            self.previous_background = None;
        } else {
            self.previous_background = Some((self.preferences.background.clone(), Instant::now()));
            self.background_fade = CYCLE_FADE;
        }
        self.preferences.background = id;
        self.dirty = true;
        true
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
            self.panel_started = None;
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
            self.panel_started = None;
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
        self.panel_started = None;
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
            Some(Subpage::TextRendering) => Target::Setting(7),
            Some(Subpage::HiddenApps) => Target::Setting(11),
            None => Target::Sidebar(self.section),
        };
        self.scroll = 0.;
        self.dirty = true;
    }

    pub fn direction(&mut self, direction: Direction, controls: &[Control]) -> Vec<Effect> {
        self.pointer = false;
        self.dirty = true;
        // A press tilts the focused card on the main screen briefly towards it.
        let tilt = self.tilt_enabled() && self.page == Page::Home && self.overlay.is_none();
        self.motion.nudge(direction, tilt);
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
            let mut ids = self.preferences.favorites(&self.inventory.apps);
            let slots: Vec<_> = ids
                .iter()
                .enumerate()
                .filter_map(|(index, id)| {
                    (!self.preferences.hidden.contains(id)
                        && self.inventory.apps.iter().any(|app| &app.id == id))
                    .then_some(index)
                })
                .collect();
            if let Some(index) = slots.iter().position(|slot| ids[*slot] == id) {
                let next = match direction {
                    Direction::Left => index.saturating_sub(1),
                    Direction::Right => (index + 1).min(slots.len() - 1),
                    _ => index,
                };
                if next != index {
                    ids.swap(slots[index], slots[next]);
                    self.preferences.favorites = Some(ids);
                    self.dirty = true;
                    return vec![Effect::Save];
                }
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
                    Some(
                        Subpage::ClockFormat
                        | Subpage::TimeFormat
                        | Subpage::Temperature
                        | Subpage::TextRendering,
                    ) => 2,
                    Some(Subpage::Language) => Language::ALL.len(),
                    // Without hidden apps the empty state holds focus.
                    Some(Subpage::HiddenApps) => self.hidden_apps().len().max(1),
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
                Target::Setting(1 | 4 | 7 | 11)
                    | Target::Widget(0..=4)
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
                            if index == 2 {
                                Target::WallpaperCycle
                            } else if index == 1 {
                                Target::WallpaperGradient
                            } else if index < 3 {
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
                    Direction::Right => self.focus(Target::WallpaperGradient, controls),
                }
                return vec![];
            }
            if self.focus == Target::WallpaperGradient {
                match direction {
                    Direction::Right => self.focus(Target::WallpaperCycle, controls),
                    Direction::Left => self.focus(Target::UploadWallpaper, controls),
                    Direction::Up => {}
                    Direction::Down => self.focus(
                        Target::Background(1.min(self.background_count().saturating_sub(1))),
                        controls,
                    ),
                }
                return vec![];
            }
            if self.focus == Target::WallpaperCycle {
                match direction {
                    Direction::Right => {}
                    Direction::Left => self.focus(Target::WallpaperGradient, controls),
                    Direction::Up => {}
                    Direction::Down => self.focus(
                        Target::Background(2.min(self.background_count().saturating_sub(1))),
                        controls,
                    ),
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
    /// and the top shelf cannot steal it: the grid (Search, apps, Settings) has six columns;
    /// input cards, when shown, are rows of six above it; the shelf's weather button is
    /// above both. `None` leaves other targets to the generic routing; `Some(None)` consumes
    /// the press without moving.
    fn home_direction(&self, direction: Direction) -> Option<Option<Target>> {
        let grid = self.grid();
        let inputs: Vec<Target> = self
            .home_inputs()
            .iter()
            .map(|input| Target::Input(input.id.clone()))
            .collect();
        let shelf = (self.preferences.show_weather && self.preferences.weather_location.is_none())
            .then_some(Target::WeatherConfigure);
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
            let row = if from_above {
                0
            } else {
                (items.len() - 1) / APPS_COLUMNS
            };
            items
                .get((row * APPS_COLUMNS + column).min(items.len() - 1))
                .cloned()
        };
        if let Some(index) = grid.iter().position(|item| *item == self.focus) {
            return Some(match step(&grid, index) {
                Ok(next) => next,
                Err(true) if !inputs.is_empty() => enter(&inputs, index % APPS_COLUMNS, false),
                Err(true) => shelf,
                Err(false) => self.inventory_failed_empty().then_some(Target::Retry),
            });
        }
        if let Some(index) = inputs.iter().position(|item| *item == self.focus) {
            return Some(match step(&inputs, index) {
                Ok(next) => next,
                Err(true) => shelf,
                Err(false) => enter(&grid, index % APPS_COLUMNS, true),
            });
        }
        if self.focus == Target::WeatherConfigure {
            return Some(match direction {
                Direction::Down if !inputs.is_empty() => inputs.first().cloned(),
                Direction::Down => Some(self.first_content()),
                _ => None,
            });
        }
        None
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
                if !self.preferences.reduced_motion {
                    self.panel_started = Some(Instant::now());
                }
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
            self.panel_started = (!self.preferences.reduced_motion).then(Instant::now);
            self.dirty = true;
        } else if let Target::App(id) = &self.focus {
            self.overlay = Some(Overlay::Options {
                id: id.clone(),
                restore: self.focus.clone(),
            });
            self.focus = Target::OpenApp;
            if !self.preferences.reduced_motion {
                self.panel_started = Some(Instant::now());
            }
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
                self.panel_started = None;
                self.pressed = None;
                self.reconcile_focus();
            }
            // Even an uncertain/failed mutation may have changed TV inventory.
            // Refresh is read-only, and also clears a stale card after late removal.
            return vec![Effect::Refresh];
        }
        let old_ids: Vec<_> = self.apps().iter().map(|app| app.id.clone()).collect();
        let index = old_ids.iter().position(|item| item == &id).unwrap_or(0);
        let mut favorites = self.preferences.favorites(&self.inventory.apps);
        favorites.retain(|item| item != &id);
        self.preferences.favorites = Some(favorites);
        self.preferences.hidden.retain(|item| item != &id);
        self.inventory.apps.retain(|app| app.id != id);
        if matches!(&self.overlay, Some(Overlay::Delete { id: current, .. } | Overlay::Options { id: current, .. }) if current == &id)
        {
            self.overlay = None;
            self.panel_started = None;
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

    /// Holding OK on a pinned favourite reorders it (as on tvOS, where icons jiggle);
    /// on any other app it opens the options popup.
    pub fn hold(&mut self) {
        if self.page == Page::Home
            && self.overlay.is_none()
            && let Target::App(id) = &self.focus
            && self
                .preferences
                .favorites(&self.inventory.apps)
                .contains(id)
        {
            self.moving = Some(id.clone());
            self.notice = None;
            self.dirty = true;
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
            self.panel_started = None;
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
    /// the end of the Home favorites and removed ones leave favorites and hidden apps.
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
                self.panel_started = None;
            }
            self.focus = Target::TileEditor;
        }
        if !added.is_empty() {
            let mut favorites = self.preferences.favorites(&self.inventory.apps);
            for id in added {
                if self.preferences.custom_tile(&id).is_some() && !favorites.contains(&id) {
                    favorites.push(id);
                }
            }
            self.preferences.favorites = Some(favorites);
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
        if field == TextField::Name {
            // SDL-webOS cannot pre-fill the system keyboard's text model.
            self.greeting_name_draft.clear();
        }
        if field == TextField::InputName {
            // The TV keyboard starts empty, so a new name replaces the old one.
            self.input_name.clear();
        }
        if cfg!(all(
            feature = "webos",
            target_os = "linux",
            target_arch = "arm"
        )) {
            self.overlay = Some(Overlay::SystemKeyboard);
            self.focus = match field {
                TextField::Location => Target::WeatherQuery,
                TextField::Name => Target::GreetingName,
                TextField::InputName => Target::InputRename,
            };
            self.panel_started = None;
            self.dirty = true;
            return;
        }
        self.overlay = Some(Overlay::Keyboard);
        self.focus = Target::Key(0);
        if !self.preferences.reduced_motion {
            self.panel_started = Some(Instant::now());
        }
        self.dirty = true;
    }
    fn open_location(&mut self) {
        self.weather.edit(String::new());
        self.subpage = Some(Subpage::WeatherLocation);
        self.focus = Target::WeatherQuery;
        self.scroll = 0.;
        if !self.preferences.reduced_motion {
            self.panel_started = Some(Instant::now());
        }
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
            Target::SettingsCard => self.open(Page::Settings),
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
                if !self.preferences.reduced_motion {
                    self.panel_started = Some(Instant::now());
                }
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
                if !self.preferences.reduced_motion {
                    self.panel_started = Some(Instant::now());
                }
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
                    self.preferences.hour_24
                } else {
                    self.preferences.show_seconds
                }));
                self.scroll = 0.;
                if !self.preferences.reduced_motion {
                    self.panel_started = Some(Instant::now());
                }
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
                    5 => {
                        self.preferences.app_artwork = !self.preferences.app_artwork;
                        return vec![Effect::Save];
                    }
                    _ => return vec![],
                };
                self.section = WIDGETS_SECTION;
                self.subpage = Some(subpage);
                self.focus = focus;
                self.scroll = 0.;
                if !self.preferences.reduced_motion {
                    self.panel_started = Some(Instant::now());
                }
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
                    self.panel_started = None;
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
                        if !self.preferences.reduced_motion {
                            self.panel_started = Some(Instant::now());
                        }
                        self.focus = Target::ClockFormat;
                        return vec![];
                    }
                    2 => self.preferences.reduced_motion = !self.preferences.reduced_motion,
                    8 => self.preferences.focus_tilt = !self.preferences.focus_tilt,
                    10 => self.preferences.favorite_badges = !self.preferences.favorite_badges,
                    11 => {
                        self.subpage = Some(Subpage::HiddenApps);
                        self.scroll = 0.;
                        self.focus = Target::Choice(0);
                        if !self.preferences.reduced_motion {
                            self.panel_started = Some(Instant::now());
                        }
                        return vec![];
                    }
                    4 => {
                        self.subpage = Some(Subpage::Language);
                        self.focus = Target::Choice(self.preferences.language.index());
                        if !self.preferences.reduced_motion {
                            self.panel_started = Some(Instant::now());
                        }
                        return vec![];
                    }
                    5 => return vec![Effect::Quit],
                    7 => {
                        self.subpage = Some(Subpage::TextRendering);
                        self.focus = Target::Choice(self.preferences.text_rendering.index());
                        self.scroll = 0.;
                        if !self.preferences.reduced_motion {
                            self.panel_started = Some(Instant::now());
                        }
                        return vec![];
                    }
                    _ => return vec![Effect::Reload],
                }
                return vec![Effect::Save];
            }
            Target::Background(_) => {
                self.options();
            }
            Target::WallpaperCycle => {
                self.preferences.wallpaper_cycle = !self.preferences.wallpaper_cycle;
                self.preferences.wallpaper_changed = unix_now();
                return vec![Effect::Save];
            }
            Target::WallpaperGradient => {
                self.preferences.wallpaper_gradient = !self.preferences.wallpaper_gradient;
                return vec![Effect::Save];
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
                if !self.preferences.reduced_motion && self.preferences.background != id {
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
                self.preferences.wallpaper_changed = unix_now();
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
                    self.panel_started = None;
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
                if !self.preferences.reduced_motion {
                    self.panel_started = Some(Instant::now());
                }
                self.focus = Target::Choice(match mode {
                    InputMode::On => 0,
                    InputMode::Off => 1,
                    InputMode::Auto => 2,
                });
            }
            Target::Choice(index) => {
                match self.subpage.clone() {
                    Some(Subpage::ClockFormat) => self.preferences.hour_24 = index == 1,
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
                    Some(Subpage::TextRendering) => {
                        let mode = TextRendering::ALL[index.min(TextRendering::ALL.len() - 1)];
                        if mode != self.preferences.text_rendering {
                            self.preferences.text_rendering = mode;
                            self.notify(self.preferences.language.t("textRenderingRestart"));
                        }
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
                        self.panel_started = None;
                    }
                    return vec![];
                }
                let Some(Overlay::Options { id, restore }) = self.overlay.take() else {
                    return vec![];
                };
                self.panel_started = None;
                let old_ids: Vec<_> = self.apps().iter().map(|a| a.id.clone()).collect();
                let old_index = old_ids.iter().position(|item| item == &id).unwrap_or(0);
                let mut favorites = self.preferences.favorites(&self.inventory.apps);
                let title = self
                    .inventory
                    .apps
                    .iter()
                    .find(|app| app.id == id)
                    .map_or(id.clone(), |app| app.title.clone());
                if index == 0 {
                    if favorites.contains(&id) {
                        favorites.retain(|item| item != &id);
                        self.notify(
                            self.preferences
                                .language
                                .format("favoriteRemoved", &[("app", &title)]),
                        );
                    } else {
                        favorites.push(id.clone());
                        self.notify(
                            self.preferences
                                .language
                                .format("favoriteAdded", &[("app", &title)]),
                        );
                    }
                    self.preferences.hidden.retain(|item| item != &id);
                    self.preferences.favorites = Some(favorites);
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
                    self.panel_started = None;
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
            return vec![];
        }
        if let Some(overlay) = self.overlay.take() {
            self.panel_started = None;
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
                self.recents.closing_focus = Some(self.focus.clone());
                self.recents.updated = Instant::now();
                self.pressed = None;
                if self.preferences.reduced_motion {
                    self.recents.visibility = 0.;
                    self.recents.closing_focus = None;
                }
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
                self.focus = Target::SettingsCard;
            } else {
                self.focus = Target::Sidebar(self.section);
            }
            self.scroll = 0.;
            return vec![];
        }
        self.open_recents()
    }
}
