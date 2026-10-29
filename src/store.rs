//! Tasks, water log and shelf items, kept on disk in the user's AppData folder.
//! Times are unix seconds; day boundaries use the local time zone.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const TODO_COLORS: [u32; 6] = [0x3BA7FF, 0x34C759, 0xFFB020, 0xFF6B5A, 0xA78BFA, 0x9AA0A6];

pub fn now() -> i64 {
    Local::now().timestamp()
}

pub fn local(ts: i64) -> DateTime<Local> {
    Local.timestamp_opt(ts, 0).single().unwrap_or_else(Local::now)
}

pub fn day_of(ts: i64) -> NaiveDate {
    local(ts).date_naive()
}

pub fn at_local(date: NaiveDate, hour: u32, minute: u32) -> i64 {
    let naive = date.and_hms_opt(hour.min(23), minute.min(59), 0).unwrap_or_default();
    Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp()).unwrap_or(0)
}

// ---------------------------------------------------------------- to-do

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Todo {
    pub id: u64,
    pub title: String,
    pub done: bool,
    #[serde(default)]
    pub color: usize,
    #[serde(default)]
    pub due: Option<i64>,
    #[serde(default)]
    pub alerted: bool,
    #[serde(default)]
    pub done_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Todos {
    pub next_id: u64,
    pub items: Vec<Todo>,
}

impl Todos {
    pub fn add(&mut self, title: &str, due: Option<i64>, color: usize) -> Option<u64> {
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(Todo {
            id,
            title: title.chars().take(200).collect(),
            done: false,
            color: color % TODO_COLORS.len(),
            due,
            alerted: false,
            done_at: None,
        });
        Some(id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Todo> {
        self.items.iter_mut().find(|t| t.id == id)
    }

    pub fn get(&self, id: u64) -> Option<&Todo> {
        self.items.iter().find(|t| t.id == id)
    }

    pub fn toggle(&mut self, id: u64, now: i64) {
        if let Some(t) = self.get_mut(id) {
            t.done = !t.done;
            t.done_at = t.done.then_some(now);
        }
    }

    pub fn rename(&mut self, id: u64, title: &str) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        if let Some(t) = self.get_mut(id) {
            t.title = title.chars().take(200).collect();
        }
    }

    pub fn set_due(&mut self, id: u64, due: Option<i64>) {
        if let Some(t) = self.get_mut(id) {
            t.due = due;
            t.alerted = false;
        }
    }

    pub fn cycle_color(&mut self, id: u64) {
        if let Some(t) = self.get_mut(id) {
            t.color = (t.color + 1) % TODO_COLORS.len();
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    pub fn snooze(&mut self, id: u64, now: i64, minutes: u32) {
        self.set_due(id, Some(now + minutes as i64 * 60));
    }

    /// Tasks whose reminder time has passed and that have not alerted yet.
    pub fn due_alerts(&self, now: i64) -> Vec<u64> {
        self.items
            .iter()
            .filter(|t| !t.done && !t.alerted && t.due.is_some_and(|d| d <= now))
            .map(|t| t.id)
            .collect()
    }

    pub fn next_due(&self, now: i64) -> Option<i64> {
        self.items.iter().filter(|t| !t.done && !t.alerted).filter_map(|t| t.due).filter(|&d| d > now).min()
    }

    pub fn on_day(&self, date: NaiveDate) -> Vec<&Todo> {
        let mut v: Vec<&Todo> = self.items.iter().filter(|t| t.due.is_some_and(|d| day_of(d) == date)).collect();
        v.sort_by_key(|t| t.due);
        v
    }

    pub fn has_tasks_on(&self, date: NaiveDate) -> bool {
        self.items.iter().any(|t| !t.done && t.due.is_some_and(|d| day_of(d) == date))
    }

    /// (done today, total that counts toward today).
    pub fn today_progress(&self, now: i64) -> (usize, usize) {
        let today = day_of(now);
        let done = self.items.iter().filter(|t| t.done && t.done_at.is_some_and(|d| day_of(d) == today)).count();
        let open = self.items.iter().filter(|t| !t.done).count();
        (done, done + open)
    }

    pub fn clear_done_before(&mut self, now: i64) {
        let today = day_of(now);
        self.items.retain(|t| !(t.done && t.done_at.is_some_and(|d| day_of(d) < today)));
    }
}

// ---------------------------------------------------------------- water

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Sip {
    pub at: i64,
    pub ml: u32,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Water {
    pub log: Vec<Sip>,
    pub snoozed_until: Option<i64>,
}

const WATER_HISTORY_DAYS: i64 = 60;

impl Water {
    pub fn drink(&mut self, now: i64, ml: u32) {
        self.log.push(Sip { at: now, ml });
        self.snoozed_until = None;
        let horizon = now - WATER_HISTORY_DAYS * 86_400;
        self.log.retain(|s| s.at >= horizon);
    }

    pub fn undo_today(&mut self, now: i64) -> bool {
        let today = day_of(now);
        match self.log.iter().rposition(|s| day_of(s.at) == today) {
            Some(i) => {
                self.log.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn total_on(&self, date: NaiveDate) -> u32 {
        self.log.iter().filter(|s| day_of(s.at) == date).map(|s| s.ml).sum()
    }

    pub fn today(&self, now: i64) -> u32 {
        self.total_on(day_of(now))
    }

    pub fn by_hour(&self, now: i64) -> [u32; 24] {
        let today = day_of(now);
        let mut hours = [0u32; 24];
        for s in self.log.iter().filter(|s| day_of(s.at) == today) {
            hours[local(s.at).hour() as usize] += s.ml;
        }
        hours
    }

    pub fn last_today(&self, now: i64) -> Option<i64> {
        let today = day_of(now);
        self.log.iter().filter(|s| day_of(s.at) == today).map(|s| s.at).max()
    }

    /// When the next reminder should fire, or None when reminders are not needed today.
    pub fn next_reminder(&self, now: i64, start_hour: u32, end_hour: u32, interval_min: u32, goal_ml: u32) -> Option<i64> {
        if self.today(now) >= goal_ml {
            return None;
        }
        let today = day_of(now);
        let window_start = at_local(today, start_hour, 0);
        let window_end = if end_hour >= 24 { at_local(today, 23, 59) } else { at_local(today, end_hour, 0) };
        if now >= window_end {
            return None;
        }
        let interval = interval_min as i64 * 60;
        let anchor = self.last_today(now).unwrap_or(window_start).max(window_start);
        let mut next = anchor + interval;
        if let Some(s) = self.snoozed_until {
            next = next.max(s);
        }
        if next > window_end {
            return None;
        }
        Some(next.max(now.min(window_end)))
    }
}

// ---------------------------------------------------------------- shelf

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ShelfItem {
    File { path: PathBuf },
    Text { text: String },
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Shelf {
    pub items: Vec<ShelfItem>,
}

pub const SHELF_LIMIT: usize = 200;

impl Shelf {
    pub fn add_paths(&mut self, paths: &[PathBuf]) -> usize {
        let mut added = 0;
        for p in paths {
            let item = ShelfItem::File { path: p.clone() };
            if !self.items.contains(&item) {
                self.items.push(item);
                added += 1;
            }
        }
        self.trim();
        added
    }

    pub fn add_text(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        let item = ShelfItem::Text { text: text.chars().take(10_000).collect() };
        if self.items.contains(&item) {
            return false;
        }
        self.items.push(item);
        self.trim();
        true
    }

    fn trim(&mut self) {
        if self.items.len() > SHELF_LIMIT {
            let extra = self.items.len() - SHELF_LIMIT;
            self.items.drain(0..extra);
        }
    }

    pub fn remove_indices(&mut self, indices: &[usize]) {
        let mut sorted: Vec<usize> = indices.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        for i in sorted.into_iter().rev() {
            if i < self.items.len() {
                self.items.remove(i);
            }
        }
    }

    /// Drops files that were moved or deleted outside SuiNotch. Returns true if anything changed.
    pub fn prune_missing(&mut self) -> bool {
        let before = self.items.len();
        self.items.retain(|i| match i {
            ShelfItem::File { path } => path.exists(),
            ShelfItem::Text { .. } => true,
        });
        before != self.items.len()
    }
}

pub fn month_grid(year: i32, month: u32, monday_first: bool) -> Vec<Option<NaiveDate>> {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
    let offset = if monday_first { first.weekday().num_days_from_monday() } else { first.weekday().num_days_from_sunday() };
    let mut cells = vec![None; offset as usize];
    let mut d = first;
    while d.month() == month {
        cells.push(Some(d));
        d = d.succ_opt().unwrap_or(d);
        if d == first {
            break;
        }
    }
    while cells.len() % 7 != 0 {
        cells.push(None);
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(h: u32, m: u32) -> i64 {
        at_local(NaiveDate::from_ymd_opt(2026, 3, 10).unwrap(), h, m)
    }

    #[test]
    fn todo_add_rejects_blank_and_tracks_progress() {
        let mut t = Todos::default();
        assert_eq!(t.add("   ", None, 0), None);
        let a = t.add("Revise the report", None, 0).unwrap();
        t.add("Reply to Dina", None, 1).unwrap();
        t.toggle(a, ts(10, 0));
        assert_eq!(t.today_progress(ts(11, 0)), (1, 2));
    }

    #[test]
    fn todo_alerts_fire_once_and_snooze_reschedules() {
        let mut t = Todos::default();
        let a = t.add("Call", Some(ts(9, 0)), 0).unwrap();
        assert!(t.due_alerts(ts(8, 59)).is_empty());
        assert_eq!(t.due_alerts(ts(9, 0)), vec![a]);
        t.get_mut(a).unwrap().alerted = true;
        assert!(t.due_alerts(ts(9, 5)).is_empty());
        t.snooze(a, ts(9, 5), 10);
        assert_eq!(t.next_due(ts(9, 5)), Some(ts(9, 15)));
        assert_eq!(t.due_alerts(ts(9, 15)), vec![a]);
    }

    #[test]
    fn done_tasks_never_alert() {
        let mut t = Todos::default();
        let a = t.add("Call", Some(ts(9, 0)), 0).unwrap();
        t.toggle(a, ts(8, 0));
        assert!(t.due_alerts(ts(10, 0)).is_empty());
        assert_eq!(t.next_due(ts(7, 0)), None);
    }

    #[test]
    fn water_totals_and_hourly_chart() {
        let mut w = Water::default();
        w.drink(ts(9, 10), 250);
        w.drink(ts(9, 50), 250);
        w.drink(ts(13, 0), 500);
        assert_eq!(w.today(ts(14, 0)), 1000);
        let h = w.by_hour(ts(14, 0));
        assert_eq!(h[9], 500);
        assert_eq!(h[13], 500);
        assert!(w.undo_today(ts(14, 0)));
        assert_eq!(w.today(ts(14, 0)), 500);
    }

    #[test]
    fn water_reminder_respects_window_interval_and_goal() {
        let mut w = Water::default();
        assert_eq!(w.next_reminder(ts(7, 0), 9, 21, 60, 2000), Some(ts(10, 0)));
        w.drink(ts(10, 30), 250);
        assert_eq!(w.next_reminder(ts(10, 31), 9, 21, 60, 2000), Some(ts(11, 30)));
        assert_eq!(w.next_reminder(ts(20, 45), 9, 21, 60, 2000), Some(ts(20, 45)));
        assert_eq!(w.next_reminder(ts(21, 30), 9, 21, 60, 2000), None);
        w.drink(ts(11, 0), 2000);
        assert_eq!(w.next_reminder(ts(11, 1), 9, 21, 60, 2000), None);
    }

    #[test]
    fn shelf_dedupes_and_removes_by_index() {
        let mut s = Shelf::default();
        let a = PathBuf::from("C:/a.txt");
        let b = PathBuf::from("C:/b.txt");
        assert_eq!(s.add_paths(&[a.clone(), b.clone(), a.clone()]), 2);
        assert!(s.add_text("hello"));
        assert!(!s.add_text("hello"));
        s.remove_indices(&[0, 2, 0]);
        assert_eq!(s.items, vec![ShelfItem::File { path: b }]);
    }

    #[test]
    fn month_grid_aligns_weekdays() {
        // March 2026 starts on a Sunday.
        let mon = month_grid(2026, 3, true);
        assert_eq!(mon.iter().position(|d| d.is_some()), Some(6));
        let sun = month_grid(2026, 3, false);
        assert_eq!(sun.iter().position(|d| d.is_some()), Some(0));
        assert_eq!(mon.iter().flatten().count(), 31);
        assert_eq!(mon.len() % 7, 0);
    }
}
