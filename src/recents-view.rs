//! The tvOS app switcher: one row of large landscape cards, the focused one centred and
//! lifted, its neighbours smaller and dimmer at the sides, painted back-to-front with a
//! separate upward exit.
use super::*;
use crate::recents::{Command, DismissalOutcome};

const RECENT_WIDTH: f32 = 840.;
/// Centre-to-centre distance between neighbouring cards.
const RECENT_PITCH: f32 = 900.;

fn card_rect(offset: f32) -> Rect {
    let depth = offset.abs();
    let scale = 1. / (1. + 0.2 * depth.min(3.));
    let width = RECENT_WIDTH * scale;
    let height = width * 9. / 16.;
    let x = 960. + offset * RECENT_PITCH;
    Rect::new(x - width / 2., 540. - height / 2., width, height)
}

pub fn render(out: &mut Scene, state: &State, assets: &Assets) {
    let language = state.preferences.language;
    let recent = &state.recents;
    let focus = recent_focus(state);
    out.modal = Some(Rect::new(0., 0., 1920., 1080.));
    out.draw.push(Draw::RoundedGradient(
        Rect::new(0., 0., 1920., 1080.),
        0.,
        c(0x252c3a, 0.18),
        c(0x0b0e15, 0.42),
    ));
    if recent.entries.is_empty() && recent.dismissals.is_empty() {
        let message = if recent.error.is_some() {
            Some("recentError")
        } else if recent.loaded {
            Some("recentEmpty")
        } else if state.recents_open() && recent.pending.is_some() {
            Some("recentLoading")
        } else {
            None
        };
        if let Some(message) = message {
            out.icon(
                assets,
                if recent.error.is_some() {
                    "alert"
                } else {
                    "recent"
                },
                Rect::new(924., 410., 72., 72.),
                c(0xffffff, 0.40),
            );
            out.text(
                language.t(message),
                Rect::new(360., 526., 1200., 60.),
                TITLE3.weight(500),
                WHITE,
                true,
            );
        }
    } else {
        let mut cards: Vec<_> = recent
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                !recent
                    .dismissals
                    .iter()
                    .any(|dismissal| dismissal.entry.app.id == entry.app.id)
            })
            .map(|(index, entry)| (entry, recent.offset(index, entry), None))
            .collect();
        for dismissal in &recent.dismissals {
            // Failed closes land at the card's current slot, even if the user
            // browsed elsewhere while the platform request was in flight.
            let offset = if dismissal.outcome == DismissalOutcome::Returning {
                let current = recent
                    .entries
                    .iter()
                    .enumerate()
                    .find(|(_, entry)| entry.app.id == dismissal.entry.app.id)
                    .map(|(index, entry)| recent.offset(index, entry))
                    .unwrap_or(dismissal.offset);
                current + (dismissal.offset - current) * dismissal.progress
            } else {
                dismissal.offset
            };
            cards.push((&dismissal.entry, offset, Some(dismissal)));
        }
        // Painting and hit testing share this order: only exposed portions of
        // a rear card can be clicked. Hover never changes depth or geometry.
        cards.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        for (entry, offset, dismissal) in cards {
            let depth = offset.abs();
            if depth > 4. {
                continue;
            }
            let mut rect = card_rect(offset);
            if let Some(dismissal) = dismissal {
                rect.y -= 1320. * dismissal.progress.powi(2);
            }
            if rect.y + rect.h + 80. < 0. || rect.x + rect.w + 64. < 0. || rect.x - 64. > 1920. {
                continue;
            }
            let start = out.draw.len();
            let target = Target::Recent(entry.app.id.clone());
            let focused = *focus == target;
            let radius = 30. * rect.w / RECENT_WIDTH;
            let prominence = (1. - depth).clamp(0., 1.);
            out.lift_shadow(rect, radius, prominence);
            let app = state
                .inventory
                .apps
                .iter()
                .find(|app| app.id == entry.app.id)
                .unwrap_or(&entry.app);
            app_face(out, assets, app, rect, language, radius);
            card_glass(out, rect, radius, if focused { prominence } else { 0. });
            if focused && dismissal.is_none() && prominence > 0. {
                let width = rect.w;
                let (title, style) = assets.single_line(&app.title, CALLOUT.weight(600), width);
                out.text(
                    title,
                    Rect::new(rect.x, rect.y + rect.h + 30., width, 38.),
                    style,
                    c(0xffffff, prominence),
                    true,
                );
            }
            // An exiting card is presentation-only, never a moving hit target.
            if dismissal.is_none() {
                out.control(target.clone(), rect, false);
            }
            if state.pressed.as_ref() == Some(&target) {
                out.fade(start, 1., 0.83);
            }
            out.fade(start, 1., 1. - 0.3 * depth.min(2.));
        }
    }

    if recent.error.is_some() {
        if !recent.entries.is_empty() {
            out.text(
                language.t("recentError"),
                Rect::new(420., 945., 1080., 32.),
                CAPTION1,
                c(0xf3ddbd, 1.),
                true,
            );
        }
        let label = language.t("retry");
        let width = assets.width(label, BODY.weight(600)) + 90.;
        button(
            out,
            state,
            assets,
            Target::RecentRetry,
            Rect::new(SAFE_X, 880., width, 68.),
            "refresh",
            label,
        );
    } else if let Some(Command::Launch(_)) = &recent.pending
        && !recent.entries.is_empty()
    {
        out.text(
            language.t("opening"),
            Rect::new(560., 945., 800., 32.),
            CAPTION1,
            c(0xffffff, 0.6),
            true,
        );
    }
}

fn recent_focus(state: &State) -> &Target {
    if state.recents_open() {
        &state.focus
    } else {
        state.recents.closing_focus.as_ref().unwrap_or(&state.focus)
    }
}

fn button(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    target: Target,
    rect: Rect,
    icon: &str,
    label: &str,
) {
    let focused = *recent_focus(state) == target;
    let busy = state.recents.pending.is_some();
    let pressed = state.pressed.as_ref() == Some(&target);
    let level = if focused && !busy && !pressed { 1. } else { 0. };
    let drawn = out.button(rect, rect.h / 2., level, false, BUTTON_LIFT);
    let color = if level > 0. {
        ON_LIGHT
    } else {
        c(0xffffff, if busy { 0.30 } else { 0.8 })
    };
    out.icon(
        assets,
        icon,
        Rect::new(drawn.x + 26., drawn.y + (drawn.h - 26.) / 2., 26., 26.),
        color,
    );
    out.text(
        label,
        Rect::new(
            drawn.x + 64.,
            drawn.y + (drawn.h - 36.) / 2.,
            drawn.w - 90.,
            36.,
        ),
        BODY.weight(600),
        color,
        false,
    );
    out.control(target, rect, false);
}
