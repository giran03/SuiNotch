//! Focus timer state machine. Times are milliseconds from a monotonic clock supplied by the caller.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Focus => "Focus time",
            Phase::ShortBreak => "Short break",
            Phase::LongBreak => "Long break",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Durations {
    pub focus_ms: u64,
    pub short_ms: u64,
    pub long_ms: u64,
    pub sessions_before_long: u32,
}

#[derive(Clone, Debug)]
pub struct Pomodoro {
    pub phase: Phase,
    pub completed_focus: u32,
    /// Remaining time while paused, or at the moment it was last started.
    remaining_ms: u64,
    /// Monotonic ms at which the running phase ends.
    ends_at: Option<u64>,
    /// Another phase picked while this timer is under way: shown, but the timer keeps going.
    preview: Option<Phase>,
}

pub fn phase_length(phase: Phase, d: &Durations) -> u64 {
    match phase {
        Phase::Focus => d.focus_ms,
        Phase::ShortBreak => d.short_ms,
        Phase::LongBreak => d.long_ms,
    }
}

impl Pomodoro {
    pub fn new(d: &Durations) -> Self {
        Pomodoro { phase: Phase::Focus, completed_focus: 0, remaining_ms: d.focus_ms, ends_at: None, preview: None }
    }

    pub fn length(&self, d: &Durations) -> u64 {
        phase_length(self.phase, d)
    }

    /// Phase picked while another timer is under way, if any.
    pub fn preview(&self) -> Option<Phase> {
        self.preview
    }

    pub fn is_running(&self) -> bool {
        self.ends_at.is_some()
    }

    pub fn is_fresh(&self, d: &Durations) -> bool {
        !self.is_running() && self.remaining_ms == self.length(d)
    }

    pub fn remaining(&self, now: u64) -> u64 {
        match self.ends_at {
            Some(end) => end.saturating_sub(now),
            None => self.remaining_ms,
        }
    }

    pub fn ends_at(&self) -> Option<u64> {
        self.ends_at
    }

    pub fn start(&mut self, now: u64) {
        if self.ends_at.is_none() {
            self.ends_at = Some(now + self.remaining_ms);
        }
    }

    pub fn pause(&mut self, now: u64) {
        if let Some(end) = self.ends_at.take() {
            self.remaining_ms = end.saturating_sub(now);
        }
    }

    /// Start/pause; on a previewed phase, starts that phase in place of the current timer.
    pub fn toggle(&mut self, now: u64, d: &Durations) {
        if let Some(phase) = self.preview {
            self.switch(phase, d);
            self.start(now);
        } else if self.is_running() {
            self.pause(now)
        } else {
            self.start(now)
        }
    }

    pub fn reset(&mut self, d: &Durations) {
        self.ends_at = None;
        self.remaining_ms = self.length(d);
        self.preview = None;
    }

    /// Makes `phase` the timer's phase, discarding any time already counted.
    pub fn switch(&mut self, phase: Phase, d: &Durations) {
        self.phase = phase;
        self.reset(d);
    }

    /// A phase chip was clicked: switch when nothing is under way, otherwise only preview it.
    pub fn select(&mut self, phase: Phase, d: &Durations) {
        if phase == self.phase {
            self.preview = None;
        } else if self.is_fresh(d) {
            self.switch(phase, d);
        } else {
            self.preview = Some(phase);
        }
    }

    /// Leaves a preview and shows the timer that is under way again.
    pub fn back(&mut self) {
        self.preview = None;
    }

    /// Settings changed: a paused untouched phase picks up the new length.
    pub fn durations_changed(&mut self, old: &Durations, new: &Durations) {
        if !self.is_running() && self.remaining_ms == self.length(old) {
            self.remaining_ms = self.length(new);
        }
    }

    fn next_phase(&self, d: &Durations) -> Phase {
        match self.phase {
            Phase::Focus if (self.completed_focus + 1) % d.sessions_before_long.max(1) == 0 => Phase::LongBreak,
            Phase::Focus => Phase::ShortBreak,
            _ => Phase::Focus,
        }
    }

    /// Moves to the next phase. `auto_start` decides whether it begins counting immediately.
    pub fn advance(&mut self, now: u64, d: &Durations, auto_start: bool) -> Phase {
        let next = self.next_phase(d);
        if self.phase == Phase::Focus {
            self.completed_focus += 1;
        }
        self.phase = next;
        self.reset(d);
        if auto_start {
            self.start(now);
        }
        next
    }

    /// Returns the finished phase once its time is up.
    pub fn poll(&mut self, now: u64) -> Option<Phase> {
        match self.ends_at {
            Some(end) if now >= end => {
                self.ends_at = None;
                self.remaining_ms = 0;
                Some(self.phase)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Durations = Durations { focus_ms: 1500, short_ms: 300, long_ms: 900, sessions_before_long: 2 };

    #[test]
    fn counts_down_and_pauses() {
        let mut p = Pomodoro::new(&D);
        p.start(1000);
        assert_eq!(p.remaining(1500), 1000);
        p.pause(1500);
        assert_eq!(p.remaining(9999), 1000);
        p.start(10_000);
        assert_eq!(p.poll(10_999), None);
        assert_eq!(p.poll(11_000), Some(Phase::Focus));
        assert!(!p.is_running());
    }

    #[test]
    fn long_break_after_configured_sessions() {
        let mut p = Pomodoro::new(&D);
        assert_eq!(p.advance(0, &D, false), Phase::ShortBreak);
        assert_eq!(p.advance(0, &D, false), Phase::Focus);
        assert_eq!(p.advance(0, &D, false), Phase::LongBreak);
        assert_eq!(p.completed_focus, 2);
        assert_eq!(p.remaining(0), 900);
    }

    #[test]
    fn auto_start_begins_next_phase() {
        let mut p = Pomodoro::new(&D);
        p.advance(100, &D, true);
        assert!(p.is_running());
        assert_eq!(p.remaining(200), 200);
    }

    #[test]
    fn selecting_a_phase_while_a_timer_runs_only_previews_it() {
        let mut p = Pomodoro::new(&D);
        p.start(0);
        p.select(Phase::ShortBreak, &D);
        assert_eq!(p.preview(), Some(Phase::ShortBreak));
        assert_eq!((p.phase, p.is_running(), p.remaining(500)), (Phase::Focus, true, 1000));
        p.select(Phase::Focus, &D);
        assert_eq!(p.preview(), None);
        assert_eq!(p.remaining(500), 1000);
    }

    #[test]
    fn paused_timer_is_kept_and_back_returns_to_it() {
        let mut p = Pomodoro::new(&D);
        p.start(0);
        p.pause(400);
        p.select(Phase::LongBreak, &D);
        p.back();
        assert_eq!((p.phase, p.preview(), p.remaining(9999)), (Phase::Focus, None, 1100));
    }

    #[test]
    fn starting_a_previewed_phase_replaces_the_timer() {
        let mut p = Pomodoro::new(&D);
        p.start(0);
        p.select(Phase::LongBreak, &D);
        p.toggle(100, &D);
        assert_eq!((p.phase, p.preview(), p.is_running(), p.remaining(100)), (Phase::LongBreak, None, true, 900));
    }

    #[test]
    fn untouched_timer_switches_straight_away() {
        let mut p = Pomodoro::new(&D);
        p.select(Phase::ShortBreak, &D);
        assert_eq!((p.phase, p.preview(), p.remaining(0)), (Phase::ShortBreak, None, 300));
    }

    #[test]
    fn new_durations_apply_to_untouched_phase_only() {
        let longer = Durations { focus_ms: 3000, ..D };
        let mut p = Pomodoro::new(&D);
        p.durations_changed(&D, &longer);
        assert_eq!(p.remaining(0), 3000);
        p.start(0);
        p.pause(500);
        p.durations_changed(&longer, &D);
        assert_eq!(p.remaining(0), 2500);
    }
}
