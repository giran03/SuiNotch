//! Calendar task context and in-memory drafts. Navigation never submits a new task.

use super::*;

#[cfg(test)]
#[path = "tasks_test.rs"]
pub(super) mod integration;

pub fn cancel_time_edit(app: &mut App) {
    if app.notch.field.as_ref().is_some_and(|f| f.target == EditTarget::ReminderTime) {
        app.notch.field = app.notch.time_return.take();
        app.notch.time_select_all = false;
    }
    app.notch.task_error = None;
}

pub fn stash_task_draft(app: &mut App) {
    cancel_time_edit(app);
    if app.notch.field.as_ref().is_some_and(|f| matches!(f.target, EditTarget::Rename(_))) {
        commit_field(app);
    }
    let n = &mut app.notch;
    if let Some(field) = n.field.take() {
        if field.target == EditTarget::NewTask && !field.text.trim().is_empty() {
            n.task_drafts.retain(|d| d.day != n.task_day);
            n.task_drafts.push(TaskDraft { field, day: n.task_day, due: n.new_due, color: n.new_color });
        }
    }
    n.new_due = None;
}

pub fn begin_task_day(app: &mut App, day: Option<NaiveDate>) {
    stash_task_draft(app);
    let n = &mut app.notch;
    n.task_day = day;
    n.due_edit = None;
    n.new_due = None;
    n.todo_scroll.snap(0.0);
    if let Some(i) = n.task_drafts.iter().position(|d| d.day == day) {
        let draft = n.task_drafts.remove(i);
        n.field = Some(draft.field);
        n.new_due = draft.due;
        n.new_color = draft.color;
    } else {
        n.field = Some(TextField { target: EditTarget::NewTask, text: String::new(), caret: 0 });
    }
}

/// Resolve local wall-clock time without silently choosing an ambiguous DST time.
pub fn reminder_at(day: NaiveDate, hour: u32, minute: u32, now: i64) -> std::result::Result<i64, &'static str> {
    use chrono::{LocalResult, TimeZone};
    let wall = day.and_hms_opt(hour, minute, 0).ok_or("Enter a valid time, such as 09:00 or 2:30 pm")?;
    let timestamp = match Local.from_local_datetime(&wall) {
        LocalResult::Single(t) => t.timestamp(),
        LocalResult::None => return Err("That local time does not exist; choose another time"),
        LocalResult::Ambiguous(_, _) => return Err("That local time occurs twice; choose another time"),
    };
    if timestamp <= now {
        return Err("Choose a future time on this day, or leave the reminder off");
    }
    Ok(timestamp)
}

pub fn default_reminder(day: NaiveDate, now: i64) -> std::result::Result<i64, &'static str> {
    let today = store::day_of(now);
    if day < today {
        return Err("Reminders are unavailable for past days");
    }
    let (hour, minute) = if day == today {
        let next = store::local(views::round_up(now + 1, 15));
        if next.date_naive() != day {
            return Err("No future quarter-hour remains today; leave the reminder off");
        }
        (next.hour(), next.minute())
    } else {
        (9, 0)
    };
    reminder_at(day, hour, minute, now)
}

pub fn parse_time(text: &str) -> Option<(u32, u32)> {
    let text = text.trim().to_uppercase();
    ["%H:%M", "%I:%M %p", "%I:%M%p", "%I %p", "%I%p"].iter()
        .find_map(|format| chrono::NaiveTime::parse_from_str(&text, format).ok())
        .map(|time| (time.hour(), time.minute()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_entry_accepts_both_clock_formats() {
        assert_eq!(parse_time("09:15"), Some((9, 15)));
        assert_eq!(parse_time("2:30 pm"), Some((14, 30)));
        assert_eq!(parse_time("12:00 am"), Some((0, 0)));
        assert_eq!(parse_time("25:00"), None);
    }

    #[test]
    fn reminders_do_not_roll_into_another_day_or_enable_in_the_past() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let now = store::at_local(day, 23, 50);
        assert!(default_reminder(day, now).is_err());
        assert!(default_reminder(day.pred_opt().unwrap(), now).is_err());
        assert!(reminder_at(day, 9, 0, now).is_err());
        let next = day.succ_opt().unwrap();
        assert_eq!(default_reminder(next, now).unwrap(), store::at_local(next, 9, 0));
    }
}
