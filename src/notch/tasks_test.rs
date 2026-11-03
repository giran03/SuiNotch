//! Opt-in native integration test; requires an isolated APPDATA directory.
use super::*;
use crate::config;

pub(crate) fn fixture() -> App {
    use std::rc::Rc;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    let isolated = config::data_dir();
    assert!(["calendar-task-test", "update-tab-test", "lyrics-toggle-test"].iter().any(|name| isolated.parent().unwrap().ends_with(name)), "Run with APPDATA pointing to a disposable calendar-task-test, update-tab-test or lyrics-toggle-test directory");
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap(); }
    let cfg = Config { respect_quiet: false, water_enabled: false, ..Config::default() };
    let durations = crate::app::durations(&cfg);
    let mut app = App {
        gfx: Rc::new(gfx::Gfx::new().unwrap()),
        notch: Notch::create(&cfg).unwrap(),
        todos: store::Todos::default(),
        water: store::Water::default(),
        shelf: store::Shelf::default(),
        audio: None,
        media: None,
        artwork: None,
        art_color: None,
        lyrics: Vec::new(),
        lyrics_key: 0,
        lyrics_source: None,
        lyrics_translation: Vec::new(),
        lyrics_romaji: Vec::new(),
        lyrics_words: Vec::new(),
        recent: crate::recent::Recent::default(),
        history: Default::default(),
        lyric_request: Default::default(),
        lyrics_attempt: 0,
        shifts: crate::lyrics::Shifts::default(),
        pomo: crate::pomodoro::Pomodoro::new(&durations),
        timer: crate::timers::Countdown::new(cfg.timer_min),
        stopwatch: crate::timers::Stopwatch::default(),
        stats: crate::stats::Stats::default(),
        clips: crate::clips::Clips::new(10),
        calendar: Vec::new(),
        calendar_error: String::new(),
        calendar_synced: None,
        calendar_alerted: Default::default(),
        settings: None,
        events: std::sync::mpsc::channel().1,
        started: Instant::now(),
        network: None,
        language: String::new(),
        caps: false,
        last_battery: None,
        last_day: today(),
        update: crate::update::Status::Idle,
        cfg,
    };
    app.notch.expanded = true;
    app
}

fn click(app: &mut App, hit: Hit) {
    views::click(app, hit, (false, false));
}

fn type_text(app: &mut App, text: &str) {
    for ch in text.encode_utf16() {
        super::super::input::on_char(app, ch);
    }
}

#[test]
#[ignore = "native graphics; requires isolated APPDATA ending in calendar-task-test"]
fn calendar_double_click_opens_the_clicked_day_and_consumes_its_release() {
    let mut app = fixture();
    let out = config::data_dir();
    std::fs::create_dir_all(&out).unwrap();
    app.notch.page = Page::Calendar;
    let day = today();
    super::super::render::task_snapshot(&mut app, &out.join("double-click-calendar.png"), 1.0).unwrap();
    let cell = app.notch.hits.iter().find(|(_, hit)| *hit == Hit::CalDay(day)).unwrap().0;
    let (x, y) = (cell.cx(), cell.cy());
    unsafe { assert_ne!(GetClassLongW(app.notch.hwnd, GCL_STYLE) & CS_DBLCLKS.0, 0); }
    super::super::input::on_button_down(&mut app, x, y);
    super::super::input::on_button_up(&mut app, x, y);
    assert_eq!(app.notch.page, Page::Calendar);
    assert_eq!(app.notch.cal_sel, day);
    super::super::input::on_double_click(&mut app, x, y);
    assert_eq!(app.notch.page, Page::Todo);
    assert_eq!(app.notch.task_day, Some(day));
    assert!(app.notch.field.as_ref().is_some_and(|f| f.target == EditTarget::NewTask));
    super::super::input::on_button_up(&mut app, x, y);
    assert!(app.todos.items.is_empty());
    assert!(app.notch.pressed.is_none());
    // Non-calendar controls still receive the second press and release.
    app.notch.hits = vec![(cell, Hit::NewColor)];
    let color = app.notch.new_color;
    super::super::input::on_double_click(&mut app, x, y);
    super::super::input::on_button_up(&mut app, x, y);
    assert_eq!(app.notch.new_color, (color + 1) % store::TODO_COLORS.len());
    unsafe { DestroyWindow(app.notch.hwnd).unwrap(); }
}

#[test]
#[ignore = "native graphics; requires isolated APPDATA ending in calendar-task-test"]
fn calendar_task_flow_preserves_dates_drafts_and_reminder_boundaries() {
    let mut app = fixture();
    let out = config::data_dir();
    std::fs::create_dir_all(&out).unwrap();
    let day = today().checked_add_signed(chrono::Duration::days(5)).unwrap();
    app.notch.page = Page::Calendar;
    app.notch.cal_sel = day;
    click(&mut app, Hit::CalAdd);
    assert_eq!(app.notch.page, Page::Todo);
    assert_eq!(app.notch.task_day, Some(day));
    assert_eq!(app.notch.field.as_ref().unwrap().target, EditTarget::NewTask);
    assert_eq!(app.notch.new_due, None);
    type_text(&mut app, "Prepare the slides");
    for scale in [1.0, 1.5, 2.0] {
        super::super::render::task_snapshot(&mut app, &out.join(format!("day-off-{scale}.png")), scale).unwrap();
        assert!(!app.notch.hits.iter().any(|(_, h)| matches!(h, Hit::DueChip(_))));
    }

    // Navigation and collapse retain the draft without creating a task.
    click(&mut app, Hit::TodoCalendar);
    type_text(&mut app, "ignored on calendar");
    assert_eq!(app.notch.cal_sel, day);
    assert_eq!(app.notch.cal_month, (day.year(), day.month()));
    app.notch.expanded = true;
    collapse(&mut app);
    assert!(app.todos.items.is_empty());
    app.notch.expanded = true;
    click(&mut app, Hit::CalAdd);
    assert_eq!(app.notch.field.as_ref().unwrap().text, "Prepare the slides");

    // Separate per-day and general drafts do not overwrite one another.
    click(&mut app, Hit::TodoDayStep(1));
    type_text(&mut app, "Next day's draft");
    click(&mut app, Hit::TodoDayStep(-1));
    assert_eq!(app.notch.field.as_ref().unwrap().text, "Prepare the slides");
    click(&mut app, Hit::TodoAll);
    type_text(&mut app, "General draft");
    begin_task_day(&mut app, Some(day));
    assert_eq!(app.notch.field.as_ref().unwrap().text, "Prepare the slides");
    commit_field(&mut app);
    assert_eq!(app.todos.on_day(day).len(), 1);
    assert_eq!(app.todos.items[0].due, None);
    assert_eq!(app.notch.task_day, Some(day));
    assert!(app.notch.field.as_ref().unwrap().text.is_empty());
    assert_eq!(app.notch.new_due, None);
    commit_field(&mut app);
    assert_eq!(app.todos.items.len(), 1);

    // A second task keeps its day, supports direct time entry, then resets reminders.
    type_text(&mut app, "Call the team");
    click(&mut app, Hit::DueEnable);
    assert_eq!(app.notch.new_due, Some(store::at_local(day, 9, 0)));
    for clock in [1, 2] {
        app.cfg.clock_format = clock;
        super::super::render::task_snapshot(&mut app, &out.join(format!("day-on-clock-{clock}.png")), 1.0).unwrap();
    }
    click(&mut app, Hit::DueTime);
    type_text(&mut app, "2:30 pm");
    commit_field(&mut app);
    assert_eq!(app.notch.new_due, Some(store::at_local(day, 14, 30)));
    assert_eq!(app.notch.field.as_ref().unwrap().text, "Call the team");
    click(&mut app, Hit::DueTime);
    type_text(&mut app, "25:90");
    commit_field(&mut app);
    assert!(app.notch.task_error.is_some());
    assert_eq!(app.notch.new_due, Some(store::at_local(day, 14, 30)));
    super::super::input::on_key(&mut app, VK_ESCAPE);
    assert_eq!(app.notch.field.as_ref().unwrap().text, "Call the team");
    app.notch.new_due = Some(store::at_local(day, 23, 50));
    click(&mut app, Hit::DueStep(1));
    assert!(app.notch.task_error.is_some());
    assert_eq!(store::day_of(app.notch.new_due.unwrap()), day);
    click(&mut app, Hit::DueClear);
    click(&mut app, Hit::DueEnable);
    click(&mut app, Hit::TodoAdd);
    assert_eq!(app.todos.on_day(day).len(), 2);
    assert_eq!(app.notch.new_due, None);

    // Editing an existing reminder does not remove the task from its day.
    let id = app.todos.items[1].id;
    click(&mut app, Hit::TodoDue(id));
    click(&mut app, Hit::DueClear);
    assert_eq!(app.todos.get(id).unwrap().day(), Some(day));
    assert!(app.todos.get(id).unwrap().due.is_none());
    assert!(app.todos.has_tasks_on(day));
    click(&mut app, Hit::TodoCalendar);
    super::super::render::task_snapshot(&mut app, &out.join("calendar.png"), 1.0).unwrap();

    // Past days accept tasks without creating an immediate alert.
    let past = today().pred_opt().unwrap();
    app.notch.cal_sel = past;
    click(&mut app, Hit::CalAdd);
    type_text(&mut app, "Record yesterday");
    click(&mut app, Hit::DueEnable);
    assert!(app.notch.task_error.is_some());
    assert!(app.notch.new_due.is_none());
    super::super::render::task_snapshot(&mut app, &out.join("past-day.png"), 1.0).unwrap();
    click(&mut app, Hit::TodoAdd);
    assert_eq!(app.todos.on_day(past).len(), 1);
    assert!(app.todos.due_alerts(store::now()).is_empty());
    let restored: store::Todos = config::load_json("todos.json");
    assert_eq!(restored.items, app.todos.items);
    // Date navigation crosses years explicitly, and All tasks restores its own draft
    // and the original relative shortcuts.
    let year_end = NaiveDate::from_ymd_opt(today().year() + 1, 12, 31).unwrap();
    begin_task_day(&mut app, Some(year_end));
    click(&mut app, Hit::TodoDayStep(1));
    assert_eq!(app.notch.task_day, NaiveDate::from_ymd_opt(year_end.year() + 1, 1, 1));
    begin_task_day(&mut app, Some(today()));
    super::super::render::task_snapshot(&mut app, &out.join("today.png"), 1.0).unwrap();
    click(&mut app, Hit::TodoAll);
    assert_eq!(app.notch.field.as_ref().unwrap().text, "General draft");
    super::super::render::task_snapshot(&mut app, &out.join("all-tasks.png"), 1.0).unwrap();
    assert_eq!(app.notch.hits.iter().filter(|(_, h)| matches!(h, Hit::DueChip(_))).count(), 4);
    unsafe { DestroyWindow(app.notch.hwnd).unwrap(); }
}
