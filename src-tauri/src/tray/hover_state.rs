//! Pure tooltip interaction rules. All clocks and pointer observations are supplied by the worker.
use std::time::{Duration, Instant};

const SHOW_DELAY: Duration = Duration::from_millis(400);
const AUTO_POP: Duration = Duration::from_secs(20);
const LEAVE_DELAY: Duration = Duration::from_millis(200);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    None,
    Show,
    Hide,
}

#[derive(Default)]
enum Phase {
    #[default]
    Hidden,
    Pending(Instant),
    Shown(Instant),
    // A click/menu/auto-pop requires an observed exit AFTER the blocking condition ends.
    Suppressed,
}

#[derive(Default)]
pub(super) struct HoverState {
    phase: Phase,
    leave_at: Option<Instant>,
}

impl HoverState {
    pub(super) fn is_shown(&self) -> bool {
        matches!(self.phase, Phase::Shown(_))
    }

    pub(super) fn reset(&mut self) -> Action {
        let action = if self.is_shown() {
            Action::Hide
        } else {
            Action::None
        };
        self.phase = Phase::Hidden;
        self.leave_at = None;
        action
    }

    pub(super) fn suppress(&mut self) -> Action {
        let action = if self.is_shown() {
            Action::Hide
        } else {
            Action::None
        };
        self.phase = Phase::Suppressed;
        self.leave_at = None;
        action
    }

    pub(super) fn observe(
        &mut self,
        now: Instant,
        inside: Option<bool>,
        on_card: bool,
        blocked: bool,
    ) -> Action {
        if blocked {
            return self.suppress();
        }
        if matches!(self.phase, Phase::Suppressed) {
            if inside == Some(false) {
                self.phase = Phase::Hidden;
            }
            return Action::None;
        }
        if let Phase::Shown(since) = self.phase {
            if on_card || inside == Some(true) {
                self.leave_at = None;
                // Reading the card keeps it alive; returning to the icon gets a fresh auto-pop interval.
                if on_card {
                    self.phase = Phase::Shown(now);
                } else if now.saturating_duration_since(since) >= AUTO_POP {
                    return self.suppress();
                }
                return Action::None;
            }
            if inside.is_none() {
                return self.reset();
            }
            let deadline = *self.leave_at.get_or_insert(now + LEAVE_DELAY);
            return if now >= deadline {
                self.reset()
            } else {
                Action::None
            };
        }
        // An unknown coordinate must never be treated as proof of a hover or exit from suppression.
        if inside != Some(true) {
            return self.reset();
        }
        match self.phase {
            Phase::Hidden => self.phase = Phase::Pending(now + SHOW_DELAY),
            Phase::Pending(deadline) if now >= deadline => {
                self.phase = Phase::Shown(now);
                return Action::Show;
            }
            _ => {}
        }
        Action::None
    }

    pub(super) fn next_wakeup(&self, now: Instant) -> Duration {
        // Poll even while hidden to recover dropped Enter/Leave events. Never busy-loop.
        let poll = Duration::from_millis(100);
        match self.phase {
            Phase::Pending(deadline) => poll.min(deadline.saturating_duration_since(now)),
            Phase::Shown(since) => {
                // During a gap only the leave deadline applies; an expired auto-pop must not spin.
                let deadline = self.leave_at.unwrap_or(since + AUTO_POP);
                poll.min(deadline.saturating_duration_since(now))
            }
            _ => poll,
        }
    }
}

pub(super) fn in_tray(pos: (f64, f64), rect: (f64, f64, f64, f64)) -> bool {
    pos.0 >= rect.0 && pos.0 < rect.0 + rect.2 && pos.1 >= rect.1 && pos.1 < rect.1 + rect.3
}

pub(super) fn in_card(pos: (f64, f64), rect: (f64, f64, f64, f64), scale: f64) -> bool {
    // The window includes an 8 CSS-pixel transparent margin, not part of the visible card.
    let inset = 8.0 * scale;
    in_tray(
        pos,
        (
            rect.0 + inset,
            rect.1 + inset,
            rect.2 - 2.0 * inset,
            rect.3 - 2.0 * inset,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    fn shown(start: Instant) -> HoverState {
        let mut state = HoverState::default();
        assert_eq!(state.observe(start, Some(true), false, false), Action::None);
        assert_eq!(
            state.observe(at(start, 400), Some(true), false, false),
            Action::Show
        );
        state
    }

    #[test]
    fn movement_does_not_restart_hover_delay() {
        let start = Instant::now();
        let mut state = HoverState::default();
        for ms in 0..400 {
            assert_eq!(
                state.observe(at(start, ms), Some(true), false, false),
                Action::None
            );
        }
        assert_eq!(
            state.observe(at(start, 400), Some(true), false, false),
            Action::Show
        );
    }

    #[test]
    fn leave_before_deadline_cancels_show() {
        let start = Instant::now();
        let mut state = HoverState::default();
        state.observe(start, Some(true), false, false);
        state.observe(at(start, 399), Some(false), false, false);
        assert_eq!(
            state.observe(at(start, 401), Some(false), false, false),
            Action::None
        );
        assert!(!state.is_shown());
    }

    #[test]
    fn crossing_gap_keeps_card_visible_for_200ms() {
        let start = Instant::now();
        let mut state = shown(start);
        assert_eq!(
            state.observe(at(start, 500), Some(false), false, false),
            Action::None
        );
        assert_eq!(
            state.observe(at(start, 699), Some(false), false, false),
            Action::None
        );
        assert_eq!(
            state.observe(at(start, 700), Some(false), false, false),
            Action::Hide
        );
    }

    #[test]
    fn quick_reentry_cancels_pending_hide_without_flicker() {
        let start = Instant::now();
        let mut state = shown(start);
        assert_eq!(
            state.observe(at(start, 410), Some(false), false, false),
            Action::None
        );
        state.observe(at(start, 420), Some(true), false, false);
        assert_eq!(
            state.observe(at(start, 820), Some(true), false, false),
            Action::None
        );
        assert_eq!(
            state.observe(at(start, 900), Some(true), false, false),
            Action::None
        );
        assert!(state.is_shown());
    }

    #[test]
    fn polling_recovers_missing_enter_and_leave_events() {
        let start = Instant::now();
        let mut state = shown(start); // observations alone, no Enter event required
        state.observe(at(start, 500), Some(false), false, false);
        assert_eq!(
            state.observe(at(start, 700), Some(false), false, false),
            Action::Hide
        );
    }

    #[test]
    fn click_requires_leave_and_reentry_not_a_fixed_cooldown() {
        let start = Instant::now();
        let mut state = shown(start);
        assert_eq!(state.suppress(), Action::Hide);
        assert_eq!(
            state.observe(at(start, 10000), Some(true), false, false),
            Action::None
        );
        state.observe(at(start, 10001), Some(false), false, false);
        state.observe(at(start, 10002), Some(true), false, false);
        assert_eq!(
            state.observe(at(start, 10402), Some(true), false, false),
            Action::Show
        );
    }

    #[test]
    fn menu_does_not_rearm_even_if_mouse_leaves_and_returns() {
        let start = Instant::now();
        let mut state = shown(start);
        assert_eq!(
            state.observe(at(start, 500), Some(true), false, true),
            Action::Hide
        );
        state.observe(at(start, 2000), Some(false), false, true);
        state.observe(at(start, 3000), Some(true), false, true);
        assert_eq!(
            state.observe(at(start, 4000), Some(true), false, false),
            Action::None
        );
        state.observe(at(start, 4100), Some(false), false, false);
        state.observe(at(start, 4200), Some(true), false, false);
        assert_eq!(
            state.observe(at(start, 4600), Some(true), false, false),
            Action::Show
        );
    }

    #[test]
    fn late_page_ready_cannot_override_leave_or_click() {
        let start = Instant::now();
        let mut state = shown(start);
        state.observe(at(start, 500), Some(false), false, false);
        state.observe(at(start, 700), Some(false), false, false);
        assert!(!state.is_shown());
        state.suppress();
        assert!(!state.is_shown()); // Ready may emit only when is_shown()
    }

    #[test]
    fn unavailable_pointer_cancels_pending_and_hides_visible_card() {
        let start = Instant::now();
        let mut state = shown(start);
        assert_eq!(
            state.observe(at(start, 500), None, false, false),
            Action::Hide
        );
        state.observe(at(start, 600), Some(true), false, false);
        state.observe(at(start, 900), None, false, false);
        assert_eq!(
            state.observe(at(start, 1000), Some(true), false, false),
            Action::None
        );
    }

    #[test]
    fn auto_pop_does_not_redisplay_until_mouse_leaves() {
        let start = Instant::now();
        let mut state = shown(start);
        assert_eq!(
            state.observe(at(start, 20400), Some(true), false, false),
            Action::Hide
        );
        assert_eq!(
            state.observe(at(start, 25000), Some(true), false, false),
            Action::None
        );
    }

    #[test]
    fn hit_test_excludes_card_padding_and_far_edge() {
        let rect = (100.0, 200.0, 24.0, 24.0);
        assert!(in_tray((100.0, 200.0), rect));
        assert!(in_tray((123.0, 223.0), rect));
        assert!(!in_tray((124.0, 212.0), rect));
        assert!(!in_tray((99.0, 212.0), rect));
        assert!(!in_tray((110.0, 100.0), rect));
    }

    #[test]
    fn click_while_pending_cancels_deadline() {
        let start = Instant::now();
        let mut state = HoverState::default();
        state.observe(start, Some(true), false, false);
        state.suppress();
        assert_eq!(
            state.observe(at(start, 500), Some(true), false, false),
            Action::None
        );
        assert!(!state.is_shown());
    }

    #[test]
    fn unknown_position_does_not_rearm_after_click() {
        let start = Instant::now();
        let mut state = shown(start);
        state.suppress();
        state.observe(at(start, 500), None, false, false);
        assert_eq!(
            state.observe(at(start, 1000), Some(true), false, false),
            Action::None
        );
    }

    #[test]
    fn wakeup_deadlines_are_serviced_during_event_streams() {
        let start = Instant::now();
        let mut state = HoverState::default();
        assert_eq!(state.next_wakeup(start), Duration::from_millis(100));
        state.observe(start, Some(true), false, false);
        assert_eq!(state.next_wakeup(at(start, 399)), Duration::from_millis(1));
        assert_eq!(
            state.observe(at(start, 400), Some(true), false, false),
            Action::Show
        );
        assert_eq!(
            state.next_wakeup(at(start, 400)),
            Duration::from_millis(100)
        );
        state.suppress();
        assert_eq!(
            state.next_wakeup(at(start, 500)),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn tray_to_card_and_back_across_gap_never_hides() {
        let start = Instant::now();
        let mut state = shown(start);
        for (ms, tray, card) in [
            (500, false, false),
            (650, false, true),
            (800, false, true),
            (1000, false, false),
            (1150, true, false),
        ] {
            assert_eq!(
                state.observe(at(start, ms), Some(tray), card, false),
                Action::None
            );
            assert!(state.is_shown());
        }
        state.observe(at(start, 1200), Some(false), false, false);
        assert_eq!(
            state.observe(at(start, 1400), Some(false), false, false),
            Action::Hide
        );
    }

    #[test]
    fn reading_card_does_not_auto_pop_and_new_exit_gets_its_own_deadline() {
        let start = Instant::now();
        let mut state = shown(start);
        assert_eq!(
            state.observe(at(start, 21000), Some(false), true, false),
            Action::None
        );
        assert_eq!(
            state.observe(at(start, 40000), Some(false), true, false),
            Action::None
        );
        assert!(state.is_shown());
        state.observe(at(start, 40010), Some(false), false, false);
        assert_eq!(
            state.next_wakeup(at(start, 40209)),
            Duration::from_millis(1)
        );
        assert_eq!(
            state.observe(at(start, 40210), Some(false), false, false),
            Action::Hide
        );
    }

    #[test]
    fn hidden_card_region_cannot_trigger_or_finish_pending_show() {
        let start = Instant::now();
        let mut state = HoverState::default();
        state.observe(start, Some(false), true, false);
        assert_eq!(
            state.observe(at(start, 500), Some(false), true, false),
            Action::None
        );
        state.observe(at(start, 600), Some(true), false, false);
        assert_eq!(
            state.observe(at(start, 1000), Some(false), true, false),
            Action::None
        );
        assert!(!state.is_shown());
    }

    #[test]
    fn menu_and_click_override_card_retention_and_gap() {
        let start = Instant::now();
        let mut state = shown(start);
        assert_eq!(
            state.observe(at(start, 500), Some(false), true, true),
            Action::Hide
        );
        assert!(!state.is_shown());
        let mut state = shown(start);
        state.observe(at(start, 500), Some(false), false, false);
        assert_eq!(state.suppress(), Action::Hide);
        assert_eq!(
            state.observe(at(start, 600), Some(true), false, false),
            Action::None
        );
    }

    #[test]
    fn card_hit_region_excludes_transparent_margin_at_all_scales() {
        for scale in [1.0, 1.25, 1.5, 2.0, 2.5] {
            let rect = (-800.0, 100.0, 380.0 * scale, 352.0 * scale);
            assert!(in_card(
                (rect.0 + 10.0 * scale, rect.1 + 10.0 * scale),
                rect,
                scale
            ));
            assert!(!in_card(
                (rect.0 + 2.0 * scale, rect.1 + 10.0 * scale),
                rect,
                scale
            ));
            assert!(!in_card(
                (rect.0 + rect.2 - 2.0 * scale, rect.1 + 10.0 * scale),
                rect,
                scale
            ));
        }
    }
}
