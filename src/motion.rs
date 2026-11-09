//! Display-only motion in the tvOS manner: the focus lift, eased scrolling and page fades.
//!
//! Logical state (focus, scroll offsets, hit regions) still changes at once; these values only
//! follow it on screen. A gained focus lifts on a light spring (about 3% overshoot, settled in
//! about 200 ms); everything else approaches its target with a critically damped exponential.
//! All of it is driven by the frame loop's `Instant`, snaps when motion is reduced, and stops
//! exactly so idle frames stop redrawing.
use crate::state::{Direction, Page, Rect, Subpage, Target};
use std::time::{Duration, Instant};

/// About 96% of the way after 200 ms (losing focus).
const FOCUS_RATE: f32 = 16.;
/// The focus spring: damping ratio 0.76 at 34 rad/s overshoots by about 2.3% and comes to rest
/// (within 0.6%) after about 0.2 s.
const SPRING_DAMPING: f32 = 0.76;
const SPRING_FREQUENCY: f32 = 34.;
/// The tilt follows its target in about 120 ms.
const TILT_RATE: f32 = 25.;
/// A d-pad press tilts the focused card towards the press for this long.
const NUDGE: Duration = Duration::from_millis(140);
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
    /// The focused card's tilt, -1..1 on each axis, towards the pointer or the last press.
    tilt: [f32; 2],
    nudge: Option<([f32; 2], Instant)>,
    /// The focused card's unscaled rectangle, from the last composed scene.
    focus_rect: Option<Rect>,
    context: Option<Context>,
    page_since: Option<Instant>,
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
    /// Whether the focused card may tilt (Focus tilt on and motion not reduced).
    pub tilt: bool,
    /// The Magic Remote pointer's logical position, while it is shown.
    pub pointer_at: Option<(f32, f32)>,
}

/// Advances an underdamped spring towards 1 in small steps; snaps once it has settled.
fn spring(value: &mut f32, velocity: &mut f32, dt: f32) -> bool {
    if *value == 1. && *velocity == 0. {
        return false;
    }
    let steps = (dt * 240.).ceil().max(1.);
    let h = dt / steps;
    for _ in 0..steps as usize {
        let acceleration = SPRING_FREQUENCY * SPRING_FREQUENCY * (1. - *value)
            - 2. * SPRING_DAMPING * SPRING_FREQUENCY * *velocity;
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
        if input.reduced {
            self.levels.retain(|(target, ..)| target == input.focus);
            for level in &mut self.levels {
                level.1 = 1.;
                level.2 = 0.;
            }
        }
        self.focus_rect = None;
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
            if input.reduced {
                changed |= *level != goal;
                *level = goal;
                *velocity = 0.;
            } else if goal == 1. {
                changed |= spring(level, velocity, dt);
            } else {
                *velocity = 0.;
                changed |= approach(level, goal, FOCUS_RATE, dt, 0.004);
            }
        }
        self.levels
            .retain(|(target, level, _)| *level > 0. || target == input.focus);

        // Tilt towards the pointer while it is over the focused card, or briefly towards a
        // d-pad press. It settles exactly, so a still pointer stops redrawing.
        let goal = if !input.tilt {
            self.nudge = None;
            [0., 0.]
        } else if let (Some((x, y)), Some(rect)) = (input.pointer_at, self.focus_rect)
            && input.pointer
            && rect.contains(x, y)
        {
            let (cx, cy) = rect.center();
            [
                ((x - cx) / (rect.w / 2.)).clamp(-1., 1.),
                ((y - cy) / (rect.h / 2.)).clamp(-1., 1.),
            ]
        } else if let Some((direction, since)) = self.nudge
            && now.duration_since(since) < NUDGE
        {
            direction
        } else {
            self.nudge = None;
            [0., 0.]
        };
        for (tilt, goal) in self.tilt.iter_mut().zip(goal) {
            if input.reduced || !input.tilt {
                changed |= *tilt != 0.;
                *tilt = 0.;
            } else {
                changed |= approach(tilt, goal, TILT_RATE, dt, 0.01);
            }
        }
        changed |= self.nudge.is_some();

        let context = Context {
            page: input.page,
            section: input.section,
            subpage: input.subpage.clone(),
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

    /// Records the focused card's resting rectangle for pointer tilt (from the composed scene).
    pub fn set_focus_rect(&mut self, rect: Option<Rect>) {
        self.focus_rect = rect;
    }

    /// Tilts the focused card briefly towards a d-pad press (ignored when tilt is off).
    pub fn nudge(&mut self, direction: Direction, enabled: bool) {
        if !enabled {
            return;
        }
        let vector = match direction {
            Direction::Up => [0., -1.],
            Direction::Down => [0., 1.],
            Direction::Left => [-1., 0.],
            Direction::Right => [1., 0.],
        };
        self.nudge = Some((vector, Instant::now()));
        self.active = true;
    }

    /// The focused card's tilt, -1..1 on each axis (0 at rest).
    pub fn tilt(&self) -> [f32; 2] {
        self.tilt
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
            scroll: 0.,
            reduced,
            pointer: false,
            tilt: !reduced,
            pointer_at: None,
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
        values.section = 1;
        motion.advance(&values, start + Duration::from_millis(32));
        assert_eq!(motion.scroll(300.), 300.);
    }

    #[test]
    fn focus_overshoots_slightly_then_settles_within_a_quarter_second() {
        let mut motion = Motion::default();
        let none = None;
        let a = Target::App("a".into());
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
    fn tilt_follows_the_pointer_and_is_inert_when_off_or_reduced() {
        let none = None;
        let a = Target::App("a".into());
        for (tilt, reduced, moves) in [
            (true, false, true),
            (false, false, false),
            (true, true, false),
        ] {
            let mut motion = Motion::default();
            let start = Instant::now();
            let mut values = input(&a, &none, reduced);
            values.tilt = tilt && !reduced;
            values.pointer = true;
            values.pointer_at = Some((230., 120.));
            motion.advance(&values, start);
            motion.set_focus_rect(Some(Rect::new(0., 0., 260., 156.)));
            motion.nudge(Direction::Left, values.tilt);
            for step in 1..30 {
                motion.advance(&values, start + Duration::from_millis(16 * step));
            }
            let [x, y] = motion.tilt();
            assert_eq!(x > 0.5 && y > 0., moves, "tilt {tilt} reduced {reduced}");
            if !moves {
                assert_eq!(motion.tilt(), [0., 0.]);
                assert!(!motion.advance(&values, start + Duration::from_secs(1)));
            }
        }
    }
}
