//! Display-only motion in the tvOS manner: the focus lift, eased scrolling and page fades.
//!
//! Logical state (focus, scroll offsets, hit regions) still changes at once; these values only
//! follow it on screen. Every value approaches its target with a critically damped exponential
//! (no overshoot), is driven by the frame loop's `Instant`, snaps when motion is reduced, and
//! stops exactly so idle frames stop redrawing.
use crate::state::{Page, Subpage, Target};
use std::time::Instant;

/// About 96% of the way after 200 ms.
const FOCUS_RATE: f32 = 16.;
/// About 96% of the way after 250 ms.
const SCROLL_RATE: f32 = 13.;
/// Page switches fade the new page in over this many seconds.
pub const PAGE_FADE: f32 = 0.2;

/// What a scroll offset belongs to; a different list starts at its own offset, not eased.
#[derive(Clone, PartialEq)]
struct Context {
    page: Page,
    section: usize,
    subpage: Option<Subpage>,
    query: String,
}

#[derive(Default)]
pub struct Motion {
    last: Option<Instant>,
    /// The focus the last `sync` saw. `None` until the frame loop first synchronizes, so scenes
    /// built without it (unit tests, idle preparation) show settled focus.
    seen: Option<Target>,
    levels: Vec<(Target, f32)>,
    scroll: Option<f32>,
    context: Option<Context>,
    page_since: Option<Instant>,
    active: bool,
}

pub struct Inputs<'a> {
    pub focus: &'a Target,
    pub page: Page,
    pub section: usize,
    pub subpage: &'a Option<Subpage>,
    pub query: &'a str,
    pub scroll: f32,
    pub reduced: bool,
    /// Wheel scrolling and pointer hover keep display and hit regions identical.
    pub pointer: bool,
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
        if !self.levels.iter().any(|(target, _)| target == input.focus) {
            let start = if input.reduced { 1. } else { 0. };
            self.levels.push((input.focus.clone(), start));
        }
        if input.reduced {
            self.levels.retain(|(target, _)| target == input.focus);
            for level in &mut self.levels {
                level.1 = 1.;
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
        for (target, level) in &mut self.levels {
            let goal = if target == input.focus { 1. } else { 0. };
            if input.reduced {
                changed |= *level != goal;
                *level = goal;
            } else {
                changed |= approach(level, goal, FOCUS_RATE, dt, 0.004);
            }
        }
        self.levels
            .retain(|(target, level)| *level > 0. || target == input.focus);

        let context = Context {
            page: input.page,
            section: input.section,
            subpage: input.subpage.clone(),
            query: input.query.to_owned(),
        };
        let switched = self.context.as_ref() != Some(&context);
        let page_switched = self
            .context
            .as_ref()
            .is_some_and(|previous| previous.page != context.page);
        self.context = Some(context);
        if page_switched && !input.reduced {
            self.page_since = Some(now);
        }
        if self.page_since.is_some_and(|since| {
            input.reduced || now.duration_since(since).as_secs_f32() >= PAGE_FADE
        }) {
            self.page_since = None;
            changed = true;
        }
        changed |= self.page_since.is_some();

        let snap = switched || input.reduced || input.pointer;
        match &mut self.scroll {
            Some(value) if !snap => changed |= approach(value, input.scroll, SCROLL_RATE, dt, 0.5),
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
            .find(|(candidate, _)| candidate == target)
            .map_or(0., |(_, level)| *level)
    }

    /// The scroll offset to draw with; hit regions keep the logical one.
    pub fn scroll(&self, logical: f32) -> f32 {
        self.scroll.unwrap_or(logical)
    }

    /// Opacity of the page content while a page switch fades it in.
    pub fn page_alpha(&self) -> f32 {
        self.page_since.map_or(1., |since| {
            let t = (since.elapsed().as_secs_f32() / PAGE_FADE).clamp(0., 1.);
            1. - (1. - t).powi(3)
        })
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
            query: "",
            scroll: 0.,
            reduced,
            pointer: false,
        }
    }

    #[test]
    fn focus_lifts_and_settles_then_stops_animating() {
        let mut motion = Motion::default();
        let none = None;
        let a = Target::App("a".into());
        let b = Target::App("b".into());
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
        assert_eq!(motion.page_alpha(), 1.);
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
        values.query = "n";
        motion.advance(&values, start + Duration::from_millis(32));
        assert_eq!(motion.scroll(300.), 300.);
    }
}
