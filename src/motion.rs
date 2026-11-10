//! Display-only motion in the tvOS manner: the focus lift and eased scrolling. Page switches
//! are instant (no fade).
//!
//! Logical state (focus, scroll offsets, hit regions) still changes at once; these values only
//! follow it on screen. A gained focus lifts on a light spring (about 3% overshoot, settled in
//! about 200 ms); everything else approaches its target with a critically damped exponential.
//! All of it is driven by the frame loop's `Instant`, snaps when motion is reduced, and stops
//! exactly so idle frames stop redrawing.
use crate::state::{Page, Subpage, Target};
use std::time::Instant;

/// About 96% of the way after 200 ms (losing focus).
const FOCUS_RATE: f32 = 16.;
/// The focus spring: damping ratio 0.76 at 34 rad/s overshoots by about 2.3% and comes to rest
/// (within 0.6%) after about 0.2 s.
const SPRING_DAMPING: f32 = 0.76;
const SPRING_FREQUENCY: f32 = 34.;
/// Small controls (top-bar icons, input glyphs, keyboard keys) respond in the frame focus moves:
/// the gaining one starts this far lifted and the losing one drops to `1 - SMALL_START` at once.
const SMALL_START: f32 = 0.4;
/// Their spring: damping 0.85 at 55 rad/s settles within 0.6% in about 100 ms, with no
/// visible overshoot.
const SMALL_DAMPING: f32 = 0.85;
const SMALL_FREQUENCY: f32 = 55.;
/// Losing focus, they fade in about 120 ms.
const SMALL_RATE: f32 = 40.;

/// Focus changes instantly, with no lift, spring or fade, on app and wallpaper cards (only a
/// highlight), on everything in Settings (its sidebar, rows, choices, buttons and keys) and in
/// every overlay (context menus, alerts, the keyboard, Recents).
pub fn instant(target: &Target, page: Page, overlay: bool) -> bool {
    overlay || page == Page::Settings || matches!(target, Target::App(_) | Target::Background(_))
}

/// Controls small enough that the cards' 200 ms lift feels sluggish on them.
pub fn small(target: &Target) -> bool {
    matches!(
        target,
        Target::StoreIcon | Target::SettingsIcon | Target::Input(_) | Target::Key(_)
    )
}

/// About 96% of the way after 250 ms.
const SCROLL_RATE: f32 = 13.;
/// Settings lists keep the focused row inside their fade quickly: about 96% after 120 ms.
const SETTINGS_SCROLL_RATE: f32 = 27.;

/// What a scroll offset belongs to; a different list starts at its own offset, not eased.
#[derive(Clone, PartialEq)]
struct Context {
    page: Page,
    section: usize,
    subpage: Option<Subpage>,
}

#[derive(Default)]
pub struct Motion {
    last: Option<Instant>,
    /// The focus the last `sync` saw. `None` until the frame loop first synchronizes, so scenes
    /// built without it (unit tests, idle preparation) show settled focus.
    seen: Option<Target>,
    /// Focus level and its velocity per element.
    levels: Vec<(Target, f32, f32)>,
    scroll: Option<f32>,
    context: Option<Context>,
    active: bool,
}

pub struct Inputs<'a> {
    pub focus: &'a Target,
    pub page: Page,
    pub section: usize,
    pub subpage: &'a Option<Subpage>,
    pub scroll: f32,
    pub reduced: bool,
    /// Wheel scrolling and pointer hover keep display and hit regions identical.
    pub pointer: bool,
    /// An overlay (menu, alert, keyboard, Recents) is open: its focus is instant too.
    pub overlay: bool,
}

/// Advances an underdamped spring towards 1 in small steps; snaps once it has settled.
fn spring(value: &mut f32, velocity: &mut f32, dt: f32, small: bool) -> bool {
    if *value == 1. && *velocity == 0. {
        return false;
    }
    let (frequency, damping) = if small {
        (SMALL_FREQUENCY, SMALL_DAMPING)
    } else {
        (SPRING_FREQUENCY, SPRING_DAMPING)
    };
    let steps = (dt * 240.).ceil().max(1.);
    let h = dt / steps;
    for _ in 0..steps as usize {
        let acceleration =
            frequency * frequency * (1. - *value) - 2. * damping * frequency * *velocity;
        *velocity += acceleration * h;
        *value += *velocity * h;
    }
    if (1. - *value).abs() < 0.006 && velocity.abs() < 0.3 {
        *value = 1.;
        *velocity = 0.;
    }
    true
}

fn approach(value: &mut f32, target: f32, rate: f32, dt: f32, epsilon: f32) -> bool {
    if (*value - target).abs() <= epsilon {
        let moved = *value != target;
        *value = target;
        return moved;
    }
    *value += (target - *value) * (1. - (-rate * dt).exp());
    if (*value - target).abs() <= epsilon {
        *value = target;
    }
    true
}

impl Motion {
    /// Registers a focus change at the moment a scene is composed, before time advances: the
    /// newly focused element starts from rest instead of flashing fully lifted for a frame.
    pub fn sync(&mut self, input: &Inputs) {
        if self.seen.as_ref() == Some(input.focus) {
            return;
        }
        self.seen = Some(input.focus.clone());
        if !self.levels.iter().any(|(target, ..)| target == input.focus) {
            let start = if input.reduced { 1. } else { 0. };
            self.levels.push((input.focus.clone(), start, 0.));
        }
        // Small controls change visibly in the very frame focus moves, before the next tick;
        // app cards change completely.
        for (target, level, velocity) in &mut self.levels {
            if instant(target, input.page, input.overlay) {
                *level = if target == input.focus { 1. } else { 0. };
                *velocity = 0.;
            } else if small(target) {
                if target == input.focus {
                    *level = level.max(SMALL_START);
                } else {
                    *level = level.min(1. - SMALL_START);
                }
            }
        }
        if input.reduced {
            self.levels.retain(|(target, ..)| target == input.focus);
            for level in &mut self.levels {
                level.1 = 1.;
                level.2 = 0.;
            }
        }
        self.active = true;
    }

    /// Advances every value towards its target. Returns whether anything visible changed.
    pub fn advance(&mut self, input: &Inputs, now: Instant) -> bool {
        self.sync(input);
        let dt = self
            .last
            .map_or(0., |last| now.duration_since(last).as_secs_f32().min(0.1));
        self.last = Some(now);
        let mut changed = false;
        for (target, level, velocity) in &mut self.levels {
            let goal = if target == input.focus { 1. } else { 0. };
            if input.reduced || instant(target, input.page, input.overlay) {
                changed |= *level != goal;
                *level = goal;
                *velocity = 0.;
            } else if goal == 1. {
                changed |= spring(level, velocity, dt, small(target));
            } else {
                *velocity = 0.;
                let rate = if small(target) {
                    SMALL_RATE
                } else {
                    FOCUS_RATE
                };
                changed |= approach(level, goal, rate, dt, 0.004);
            }
        }
        self.levels
            .retain(|(target, level, _)| *level > 0. || target == input.focus);

        let context = Context {
            page: input.page,
            section: input.section,
            subpage: input.subpage.clone(),
        };
        // A page switch is instant: the new page's first frame is already complete (no fade
        // that would show the bare wallpaper for a frame), and its scroll starts in place.
        let switched = self.context.as_ref() != Some(&context);
        self.context = Some(context);

        let snap = switched || input.reduced || input.pointer;
        match &mut self.scroll {
            Some(value) if !snap => {
                let rate = if input.page == Page::Settings {
                    SETTINGS_SCROLL_RATE
                } else {
                    SCROLL_RATE
                };
                changed |= approach(value, input.scroll, rate, dt, 0.5)
            }
            display => {
                changed |= display.is_some_and(|value| value != input.scroll);
                *display = Some(input.scroll);
            }
        }
        self.active = changed;
        changed
    }

    pub fn animating(&self) -> bool {
        self.active
    }

    /// 0 at rest, 1 fully lifted.
    pub fn focus_level(&self, target: &Target, focus: &Target) -> f32 {
        if self.seen.is_none() {
            return if target == focus { 1. } else { 0. };
        }
        self.levels
            .iter()
            .find(|(candidate, ..)| candidate == target)
            .map_or(0., |(_, level, _)| *level)
    }

    /// The scroll offset to draw with; hit regions keep the logical one.
    pub fn scroll(&self, logical: f32) -> f32 {
        self.scroll.unwrap_or(logical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn input<'a>(focus: &'a Target, subpage: &'a Option<Subpage>, reduced: bool) -> Inputs<'a> {
        Inputs {
            focus,
            page: Page::Home,
            section: 0,
            subpage,
            scroll: 0.,
            reduced,
            pointer: false,
            overlay: false,
        }
    }

    #[test]
    fn focus_lifts_and_settles_then_stops_animating() {
        let mut motion = Motion::default();
        let none = None;
        let a = Target::Setting(0);
        let b = Target::Setting(1);
        let start = Instant::now();
        motion.advance(&input(&a, &none, false), start);
        motion.sync(&input(&b, &none, false));
        assert_eq!(motion.focus_level(&b, &b), 0.);
        let mut now = start;
        for _ in 0..30 {
            now += Duration::from_millis(16);
            motion.advance(&input(&b, &none, false), now);
        }
        assert_eq!(motion.focus_level(&b, &b), 1.);
        assert_eq!(motion.focus_level(&a, &b), 0.);
        now += Duration::from_millis(16);
        assert!(!motion.advance(&input(&b, &none, false), now));
        assert!(!motion.animating());
    }

    #[test]
    fn reduced_motion_snaps_focus_and_scroll() {
        let mut motion = Motion::default();
        let none = None;
        let a = Target::App("a".into());
        let mut values = input(&a, &none, true);
        motion.advance(&values, Instant::now());
        values.scroll = 300.;
        motion.advance(&values, Instant::now());
        assert_eq!(motion.scroll(300.), 300.);
        assert_eq!(motion.focus_level(&a, &a), 1.);
    }

    #[test]
    fn scroll_eases_within_a_list_but_snaps_to_a_new_list() {
        let mut motion = Motion::default();
        let none = None;
        let a = Target::App("a".into());
        let start = Instant::now();
        let mut values = input(&a, &none, false);
        motion.advance(&values, start);
        values.scroll = 300.;
        motion.advance(&values, start + Duration::from_millis(16));
        let display = motion.scroll(300.);
        assert!(display > 0. && display < 300.);
        values.section = 1;
        motion.advance(&values, start + Duration::from_millis(32));
        assert_eq!(motion.scroll(300.), 300.);
    }

    #[test]
    fn focus_overshoots_slightly_then_settles_within_a_quarter_second() {
        // Rows and buttons keep the spring; app cards change instantly.
        let mut motion = Motion::default();
        let none = None;
        let a = Target::Setting(0);
        let start = Instant::now();
        motion.advance(&input(&a, &none, false), start);
        let mut peak: f32 = 0.;
        let mut now = start;
        for _ in 0..15 {
            now += Duration::from_millis(16);
            motion.advance(&input(&a, &none, false), now);
            peak = peak.max(motion.focus_level(&a, &a));
        }
        // Sampled per frame, so the observed peak sits a little under the true 2.3%.
        assert!((1.01..1.04).contains(&peak), "peak {peak}");
        assert_eq!(motion.focus_level(&a, &a), 1.);
    }

    #[test]
    fn small_controls_respond_in_the_first_frame_and_settle_in_about_120_ms() {
        let none = None;
        let settings = Target::SettingsIcon;
        let control = Target::StoreIcon;
        let card = Target::App("a".into());
        let mut motion = Motion::default();
        let start = Instant::now();
        motion.advance(&input(&settings, &none, false), start);
        let mut now = start;
        for _ in 0..20 {
            now += Duration::from_millis(16);
            motion.advance(&input(&settings, &none, false), now);
        }
        assert_eq!(motion.focus_level(&settings, &settings), 1.);
        // The press: synchronised before any tick, the change is already visible.
        motion.sync(&input(&control, &none, false));
        assert!(motion.focus_level(&control, &control) >= SMALL_START);
        assert!(motion.focus_level(&settings, &control) <= 1. - SMALL_START);
        let mut settled = None;
        for frame in 1..=12 {
            now += Duration::from_millis(16);
            motion.advance(&input(&control, &none, false), now);
            if settled.is_none()
                && motion.focus_level(&control, &control) == 1.
                && motion.focus_level(&settings, &control) == 0.
            {
                settled = Some(frame * 16);
            }
        }
        let settled = settled.expect("small controls settle");
        assert!(settled <= 144, "settled after {settled} ms");
        // App cards change completely in that frame, and the card they leave at once.
        motion.sync(&input(&card, &none, false));
        assert_eq!(motion.focus_level(&card, &card), 1.);
        assert_eq!(motion.focus_level(&control, &card), 1. - SMALL_START);
        let other = Target::App("b".into());
        motion.sync(&input(&other, &none, false));
        assert_eq!(motion.focus_level(&card, &other), 0.);
        assert_eq!(motion.focus_level(&other, &other), 1.);
    }
}
