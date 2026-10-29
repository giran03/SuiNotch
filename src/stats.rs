//! Daily totals behind the Stats page: focus minutes and sessions, and tasks finished.
//! Water comes from the water log itself. Kept in stats.json, keyed by local date.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

const KEEP_DAYS: i64 = 400;

#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Day {
    pub focus_min: u32,
    pub sessions: u32,
    pub tasks_done: u32,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Stats {
    /// "YYYY-MM-DD" to that day's totals.
    pub days: BTreeMap<String, Day>,
}

fn key(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

impl Stats {
    pub fn day(&self, d: NaiveDate) -> Day {
        self.days.get(&key(d)).copied().unwrap_or_default()
    }

    fn entry(&mut self, d: NaiveDate) -> &mut Day {
        let horizon = key(d - chrono::Duration::days(KEEP_DAYS));
        self.days.retain(|k, _| *k >= horizon);
        self.days.entry(key(d)).or_default()
    }

    pub fn add_focus(&mut self, d: NaiveDate, minutes: u32) {
        let day = self.entry(d);
        day.focus_min += minutes;
        day.sessions += 1;
    }

    /// A task was ticked (`done`) or unticked on day `d`.
    pub fn task_done(&mut self, d: NaiveDate, done: bool) {
        let day = self.entry(d);
        day.tasks_done = if done { day.tasks_done + 1 } else { day.tasks_done.saturating_sub(1) };
    }

    /// The seven days ending on `end`, oldest first.
    pub fn week(&self, end: NaiveDate) -> [(NaiveDate, Day); 7] {
        std::array::from_fn(|i| {
            let d = end - chrono::Duration::days(6 - i as i64);
            (d, self.day(d))
        })
    }
}

/// Consecutive days where `hit` holds, ending today, or yesterday while today is still open.
pub fn streak(today: NaiveDate, hit: impl Fn(NaiveDate) -> bool) -> u32 {
    let mut d = if hit(today) { today } else { today - chrono::Duration::days(1) };
    let mut n = 0;
    while hit(d) {
        n += 1;
        d -= chrono::Duration::days(1);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    #[test]
    fn focus_and_tasks_add_up_per_day() {
        let mut s = Stats::default();
        s.add_focus(date(29), 25);
        s.add_focus(date(29), 25);
        s.task_done(date(29), true);
        s.task_done(date(29), true);
        s.task_done(date(29), false);
        s.task_done(date(30), false);
        assert_eq!(s.day(date(29)), Day { focus_min: 50, sessions: 2, tasks_done: 1 });
        assert_eq!(s.day(date(30)).tasks_done, 0);
        let week = s.week(date(30));
        assert_eq!((week[0].0, week[5].1.focus_min, week[6].0), (date(24), 50, date(30)));
    }

    #[test]
    fn streak_counts_back_and_forgives_an_open_today() {
        let hits = [date(26), date(27), date(28), date(29)];
        let hit = |d: NaiveDate| hits.contains(&d);
        assert_eq!(streak(date(29), hit), 4);
        // Today has nothing yet: yesterday's run still counts.
        assert_eq!(streak(date(30), hit), 4);
        // A missed day breaks it.
        assert_eq!(streak(date(31), hit), 0);
    }

    #[test]
    fn old_days_are_dropped() {
        let mut s = Stats::default();
        s.add_focus(date(1) - chrono::Duration::days(500), 25);
        s.add_focus(date(1), 25);
        assert_eq!(s.days.len(), 1);
    }
}
