#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Phase {
    #[default]
    Windowed,
    Entering,
    Fullscreen,
    Exiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Hide,
    ExitFullscreen,
}

#[derive(Default)]
pub(super) struct VisibilityState {
    pub(super) phase: Phase,
    pending: Option<u64>,
    generation: u64,
    exit_requested: bool,
}

impl VisibilityState {
    /// Returns a ticket only for a new request, coalescing repeated Close/Cmd+Q.
    pub(super) fn request_hide(&mut self, fullscreen: bool) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        if self.phase == Phase::Windowed && fullscreen {
            self.phase = Phase::Fullscreen;
        }
        self.generation = self.generation.wrapping_add(1);
        self.pending = Some(self.generation);
        self.pending
    }

    pub(super) fn observe(&mut self, phase: Phase) {
        self.phase = phase;
        if matches!(phase, Phase::Windowed | Phase::Fullscreen) {
            self.exit_requested = false;
        }
    }

    pub(super) fn cancel(&mut self) {
        self.pending = None;
    }

    pub(super) fn abort(&mut self) {
        self.cancel();
        self.exit_requested = false;
    }

    pub(super) fn expire(&mut self, ticket: u64) -> bool {
        if self.pending == Some(ticket) {
            self.abort();
            true
        } else {
            false
        }
    }

    /// Call on the main thread immediately before the native action. A reopen
    /// or a new transition between the notification and this call wins.
    pub(super) fn take_action(&mut self) -> Option<Action> {
        self.pending?;
        match self.phase {
            Phase::Windowed => {
                self.pending = None;
                Some(Action::Hide)
            }
            Phase::Fullscreen if !self.exit_requested => {
                self.exit_requested = true;
                Some(Action::ExitFullscreen)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_hide_waits_for_native_exit_completion() {
        let mut state = VisibilityState::default();
        state.request_hide(true).unwrap();
        assert_eq!(state.take_action(), Some(Action::ExitFullscreen));
        assert_eq!(state.take_action(), None);
        state.observe(Phase::Exiting);
        assert_eq!(state.take_action(), None);
        state.observe(Phase::Windowed);
        assert_eq!(state.take_action(), Some(Action::Hide));
        assert_eq!(state.take_action(), None);
    }

    #[test]
    fn close_during_entry_defers_exit_until_entry_completes() {
        let mut state = VisibilityState::default();
        state.observe(Phase::Entering);
        state.request_hide(false).unwrap();
        assert_eq!(state.take_action(), None);
        state.observe(Phase::Fullscreen);
        assert_eq!(state.take_action(), Some(Action::ExitFullscreen));
        state.observe(Phase::Exiting);
        assert_eq!(state.take_action(), None);
        state.observe(Phase::Windowed);
        assert_eq!(state.take_action(), Some(Action::Hide));
    }

    #[test]
    fn reopen_cancels_pending_hide_even_after_exit_notification() {
        for phase in [
            Phase::Entering,
            Phase::Fullscreen,
            Phase::Exiting,
            Phase::Windowed,
        ] {
            let mut state = VisibilityState::default();
            state.request_hide(true).unwrap();
            state.observe(phase);
            state.cancel();
            assert_eq!(state.take_action(), None);
            state.observe(Phase::Windowed);
            assert_eq!(state.take_action(), None);
        }
    }

    #[test]
    fn timeout_never_hides_and_cannot_cancel_a_new_request() {
        let mut state = VisibilityState::default();
        let old = state.request_hide(true).unwrap();
        assert_eq!(state.request_hide(true), None);
        assert!(state.expire(old));
        state.observe(Phase::Windowed);
        assert_eq!(state.take_action(), None);
        let new = state.request_hide(false).unwrap();
        assert_ne!(old, new);
        assert!(!state.expire(old));
        assert_eq!(state.take_action(), Some(Action::Hide));
    }

    #[test]
    fn failed_exit_request_can_be_retried_without_hiding() {
        let mut state = VisibilityState::default();
        state.request_hide(true).unwrap();
        assert_eq!(state.take_action(), Some(Action::ExitFullscreen));
        state.abort();
        assert_eq!(state.take_action(), None);
        state.request_hide(true).unwrap();
        assert_eq!(state.take_action(), Some(Action::ExitFullscreen));
    }

    #[test]
    fn five_fullscreen_close_reopen_cycles_do_not_reuse_pending_hide() {
        let mut state = VisibilityState::default();
        for _ in 0..5 {
            state.observe(Phase::Entering);
            state.observe(Phase::Fullscreen);
            state.request_hide(true).unwrap();
            assert_eq!(state.take_action(), Some(Action::ExitFullscreen));
            state.observe(Phase::Exiting);
            state.observe(Phase::Windowed);
            assert_eq!(state.take_action(), Some(Action::Hide));
            state.cancel();
            assert_eq!(state.take_action(), None);
        }
        state.request_hide(false).unwrap();
        assert_eq!(state.take_action(), Some(Action::Hide));
    }
}
