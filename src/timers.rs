//! A plain countdown and a stopwatch for the Focus tab, beside the Pomodoro.
//! Times are milliseconds from a monotonic clock supplied by the caller.

pub const PRESETS_MIN: [u32; 5] = [1, 3, 5, 10, 15];
pub const MAX_MIN: u32 = 180;
const MAX_LAPS: usize = 20;

#[derive(Clone, Debug)]
pub struct Countdown {
    pub length_ms: u64,
    /// Remaining time while paused, or at the moment it was last started.
    remaining_ms: u64,
    ends_at: Option<u64>,
}

impl Countdown {
    pub fn new(minutes: u32) -> Self {
        let length_ms = minutes.clamp(1, MAX_MIN) as u64 * 60_000;
        Countdown { length_ms, remaining_ms: length_ms, ends_at: None }
    }

    pub fn is_running(&self) -> bool {
        self.ends_at.is_some()
    }

    pub fn is_fresh(&self) -> bool {
        !self.is_running() && self.remaining_ms == self.length_ms
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

    pub fn toggle(&mut self, now: u64) {
        match self.ends_at.take() {
            Some(end) => self.remaining_ms = end.saturating_sub(now),
            None => {
                if self.remaining_ms == 0 {
                    self.remaining_ms = self.length_ms;
                }
                self.ends_at = Some(now + self.remaining_ms);
            }
        }
    }

    pub fn reset(&mut self) {
        self.ends_at = None;
        self.remaining_ms = self.length_ms;
    }

    /// Picks a new length; only an untouched timer changes, so a running one is never lost.
    pub fn set_minutes(&mut self, minutes: u32) -> bool {
        if !self.is_fresh() {
            return false;
        }
        *self = Countdown::new(minutes);
        true
    }

    pub fn minutes(&self) -> u32 {
        (self.length_ms / 60_000) as u32
    }

    /// True once, when the running countdown reaches zero.
    pub fn poll(&mut self, now: u64) -> bool {
        match self.ends_at {
            Some(end) if now >= end => {
                self.ends_at = None;
                self.remaining_ms = 0;
                true
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stopwatch {
    /// Time counted before the current run started.
    banked_ms: u64,
    started_at: Option<u64>,
    /// Lap end times, newest last.
    pub laps: Vec<u64>,
}

impl Stopwatch {
    pub fn is_running(&self) -> bool {
        self.started_at.is_some()
    }

    pub fn elapsed(&self, now: u64) -> u64 {
        self.banked_ms + self.started_at.map_or(0, |s| now.saturating_sub(s))
    }

    pub fn toggle(&mut self, now: u64) {
        match self.started_at.take() {
            Some(s) => self.banked_ms += now.saturating_sub(s),
            None => self.started_at = Some(now),
        }
    }

    pub fn lap(&mut self, now: u64) {
        if self.is_running() {
            self.laps.push(self.elapsed(now));
            if self.laps.len() > MAX_LAPS {
                self.laps.remove(0);
            }
        }
    }

    /// Length of lap `i` (0 = first), from the end of the previous one.
    pub fn lap_length(&self, i: usize) -> u64 {
        let prev = if i == 0 { 0 } else { self.laps[i - 1] };
        self.laps[i].saturating_sub(prev)
    }

    pub fn reset(&mut self) {
        *self = Stopwatch::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_runs_pauses_and_fires_once() {
        let mut t = Countdown::new(1);
        t.toggle(0);
        assert_eq!(t.remaining(15_000), 45_000);
        t.toggle(15_000);
        assert_eq!(t.remaining(99_000), 45_000);
        t.toggle(100_000);
        assert!(!t.poll(144_999));
        assert!(t.poll(145_000));
        assert!(!t.poll(146_000));
        // Starting again after it finished runs the full length.
        t.toggle(200_000);
        assert_eq!(t.remaining(200_000), 60_000);
    }

    #[test]
    fn length_changes_only_while_untouched() {
        let mut t = Countdown::new(5);
        assert!(t.set_minutes(10));
        assert_eq!(t.minutes(), 10);
        t.toggle(0);
        assert!(!t.set_minutes(3));
        assert_eq!(t.remaining(0), 600_000);
        assert_eq!(Countdown::new(0).minutes(), 1);
        assert_eq!(Countdown::new(999).minutes(), MAX_MIN);
    }

    #[test]
    fn stopwatch_banks_time_and_records_laps() {
        let mut s = Stopwatch::default();
        s.lap(0);
        assert!(s.laps.is_empty());
        s.toggle(1_000);
        s.lap(3_000);
        s.toggle(4_000);
        assert_eq!(s.elapsed(50_000), 3_000);
        s.toggle(10_000);
        s.lap(12_000);
        assert_eq!(s.laps, vec![2_000, 5_000]);
        assert_eq!((s.lap_length(0), s.lap_length(1)), (2_000, 3_000));
        s.reset();
        assert_eq!(s.elapsed(99_000), 0);
    }
}
