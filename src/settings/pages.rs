//! The content of each settings page, laid out with the controls in `controls`.

use super::*;

impl Ui<'_, '_> {
    pub(super) fn page(&mut self, app: &App) {
        let cfg = app.cfg.clone();
        let scroll = self.s.scroll.value;
        let clip = R::new(SIDEBAR + 1.0, TITLE_H - 8.0, W - SIDEBAR - 1.0, H - TITLE_H + 8.0);
        self.cv.push_clip_rrect(clip, 0.0, 1.0);
        self.y = TITLE_H - 10.0 - scroll;
        let start = self.y;
        self.row_index = 0;
        match self.s.page {
            0 => self.general(&cfg),
            1 => self.display(&cfg),
            2 => self.appearance(&cfg),
            3 => self.music(&cfg),
            4 => self.todo(&cfg),
            5 => self.calendar(&cfg),
            6 => self.pomodoro(&cfg),
            7 => self.water(&cfg),
            8 => self.shelf(&cfg, app.shelf.items.len()),
            9 => self.hud(&cfg),
            CHANGELOG_PAGE => self.changelog(),
            _ => self.about(&cfg, &app.update),
        }
        self.s.content_h = self.y - start;
        self.cv.pop();
        // Keep hits from scrolled-out content from stealing clicks in the title area.
        self.s.hits.retain(|(r, a)| matches!(a, Act::Page(_)) || r.bottom() > TITLE_H - 8.0);
    }

    fn general(&mut self, c: &Config) {
        self.title("General");
        self.section("Startup");
        let r = self.card(&[ROW]);
        self.toggle(c, r[0], "Start with Windows", None, |c| &mut c.start_with_windows);
        self.section("Notch");
        let r = self.card(&[86.0, ROW_SUB, ROW_SUB]);
        self.seg_row(c, r[0], "Size", Some("Applies right away. Auto is larger on wide monitors"), &config::SIZE_LABELS, |c| &mut c.size);
        self.toggle(c, r[1], "Minimal notch", Some("A thin line when nothing is playing"), |c| &mut c.minimal);
        self.toggle(c, r[2], "Sui mascot", Some("A little droplet that lives in the notch"), |c| &mut c.mascot);
        self.section("Sounds");
        let r = self.card(&[ROW_SUB]);
        self.toggle(c, r[0], "Reminder sounds", Some("Chimes for Pomodoro, To-do and Water"), |c| &mut c.sounds);
        self.section("Full screen");
        let r = self.card(&[ROW_SUB]);
        self.toggle(c, r[0], "Hide in full screen", Some("Games, videos and slides that fill the screen"), |c| &mut c.hide_fullscreen);
    }

    fn display(&mut self, c: &Config) {
        self.title("Display");
        self.section("Show the notch on");
        let monitors = self.s.monitors.clone();
        let mut heights = vec![ROW];
        heights.extend(std::iter::repeat_n(ROW, monitors.len()));
        let rows = self.card(&heights);
        let accent = Self::accent(c);
        let auto = c.display.is_none() || !monitors.iter().any(|m| Some(&m.device) == c.display.as_ref());
        self.radio(rows[0], "Main display", Some("Follows the primary monitor"), auto, Act::Display(None), accent);
        for (i, m) in monitors.iter().enumerate() {
            let size = format!("{} x {}", m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
            let name = format!("Display {}{}", i + 1, if m.primary { " (primary)" } else { "" });
            let selected = !auto && c.display.as_deref() == Some(m.device.as_str());
            self.radio_with_meta(rows[i + 1], &name, &size, selected, Act::Display(Some(i)), accent);
        }
        self.section("Behaviour");
        let r = self.card(&[ROW_SUB, 86.0]);
        self.toggle(c, r[0], "Open on hover", Some("Otherwise click the notch to open it"), |c| &mut c.expand_on_hover);
        self.seg_row(c, r[1], "Hover delay", Some("How long to rest on the notch before it opens"), &config::HOVER_DELAY_LABELS, |c| &mut c.hover_delay);
    }

    fn radio(&mut self, r: R, label: &str, sub: Option<&str>, on: bool, act: Act, accent: C) {
        self.s.hits.push((r, act));
        if hovered(self.s, act) {
            self.cv.fill_rrect(r.inset(4.0, 3.0), 9.0, C::WHITE.a(0.03));
        }
        self.label(r, label, sub);
        let (cx, cy) = (r.right() - 30.0, r.cy());
        if on {
            self.cv.fill_circle(cx, cy, 10.0, accent);
            self.cv.glyph(icon::CHECK, cx, cy, 10.0, crate::views::readable_on(accent));
        } else {
            self.cv.stroke_circle(cx, cy, 9.0, C::WHITE.a(0.3), 1.5);
        }
    }

    fn radio_with_meta(&mut self, r: R, label: &str, meta: &str, on: bool, act: Act, accent: C) {
        self.radio(r, label, None, on, act, accent);
        self.cv.text(meta, R::new(r.right() - 220.0, r.y, 170.0, r.h), 12.0, 500, DIM, Align::Right);
    }

    fn appearance(&mut self, c: &Config) {
        self.title("Appearance");
        self.section("Accent colour");
        let r = self.card(&[64.0]);
        let row = r[0];
        for (i, rgb) in ACCENTS.iter().enumerate() {
            let cx = row.x + 32.0 + i as f32 * 46.0;
            let act = Act::Accent(i);
            self.s.hits.push((R::centered(cx, row.cy(), 40.0, 40.0), act));
            let col = C::hex(*rgb);
            let grow = if hovered(self.s, act) { 1.5 } else { 0.0 };
            self.cv.fill_circle(cx, row.cy(), 13.0 + grow, col);
            if c.accent == i {
                self.cv.stroke_circle(cx, row.cy(), 18.0, col, 2.0);
            }
        }
        self.section("Shape");
        let r = self.card(&[86.0, ROW_SUB, ROW_SUB]);
        self.seg_row(c, r[0], "Corner roundness", Some("How soft the bottom corners of the notch are"), &config::ROUNDNESS_LABELS, |c| &mut c.roundness);
        self.toggle(c, r[1], "Frosted glass", Some("A slightly translucent notch with a fine edge"), |c| &mut c.glass);
        self.toggle(c, r[2], "Shadow", Some("A soft shadow under the open notch"), |c| &mut c.shadow);
        self.section("Motion");
        let r = self.card(&[86.0]);
        self.seg_row(c, r[0], "Animation style", Some("Spring feel for opening, closing and switching pages"), &config::ANIMATION_LABELS, |c| &mut c.animation);
        self.section("Clock");
        let r = self.card(&[86.0]);
        self.seg_row(c, r[0], "Time format", Some("System follows your Windows region settings"), &config::CLOCK_LABELS, |c| &mut c.clock_format);
    }

    fn music(&mut self, c: &Config) {
        self.title("Music and lyrics");
        self.section("Lyrics");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Open lyrics automatically", Some("Show the lyrics panel whenever the Music page opens"), |c| &mut c.lyrics_auto);
        self.toggle(c, r[1], "Lyrics under the notch", Some("Each line lights up in a pill below the closed notch"), |c| &mut c.lyrics_pill);
        self.section("Now playing");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Equalizer animation", Some("Moving bars in the closed notch while music plays"), |c| &mut c.equalizer);
        self.toggle(c, r[1], "Artwork glow", Some("Tint the notch with the colours of the album cover"), |c| &mut c.artwork_glow);
        self.toggle(c, r[2], "Show player name", Some("Spotify, Apple Music, Chrome and others"), |c| &mut c.show_source);
        self.section("Privacy");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], "Lyrics come from LRCLIB", Some("Only the song title, artist, album and length are sent"), "lrclib.net", Act::Website, false);
    }

    fn todo(&mut self, c: &Config) {
        self.title("To-do");
        self.section("Reminders");
        let r = self.card(&[86.0]);
        self.seg_row(c, r[0], "Snooze for", Some("When a \"Time's up!\" reminder is snoozed"), &config::SNOOZE_LABELS, |c| &mut c.todo_snooze);
        self.section("List");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Show completed tasks", Some("Keep finished tasks visible, crossed out"), |c| &mut c.todo_show_done);
        self.toggle(c, r[1], "Clear completed tasks daily", Some("Finished tasks disappear the next morning"), |c| &mut c.todo_clear_done_daily);
    }

    fn calendar(&mut self, c: &Config) {
        self.title("Calendar");
        self.section("Calendar page");
        let r = self.card(&[ROW_SUB, 86.0, ROW_SUB]);
        self.toggle(c, r[0], "Show the Calendar page", Some("A month view of your tasks in the notch"), |c| &mut c.calendar_page);
        let mut idx = if c.week_starts_monday { 0usize } else { 1 };
        let _ = &mut idx;
        self.label(R::new(r[1].x, r[1].y, r[1].w, 38.0).offset(0.0, 4.0), "Week starts on", None);
        let bar = R::new(r[1].x + 16.0, r[1].bottom() - 42.0, r[1].w - 32.0, 30.0);
        self.week_seg(c, bar);
        self.toggle(c, r[2], "Task dots", Some("Mark days that have tasks"), |c| &mut c.calendar_dots);
    }

    fn week_seg(&mut self, c: &Config, bar: R) {
        let monday = c.week_starts_monday;
        self.cv.fill_rrect(bar, 8.0, C::WHITE.a(0.06));
        let half = bar.w / 2.0;
        let sel = if monday { 0 } else { 1 };
        self.cv.fill_rrect(R::new(bar.x + 2.0 + sel as f32 * half, bar.y + 2.0, half - 4.0, bar.h - 4.0), 6.5, C::WHITE.a(0.16));
        for (i, l) in ["Monday", "Sunday"].iter().enumerate() {
            let sr = R::new(bar.x + i as f32 * half, bar.y, half, bar.h);
            if i != sel {
                self.s.hits.push((sr, Act::Toggle(|c| &mut c.week_starts_monday)));
            }
            self.cv.text(l, sr, 12.0, if i == sel { 650 } else { 500 }, if i == sel { C::WHITE } else { DIM }, Align::Center);
        }
    }

    fn pomodoro(&mut self, c: &Config) {
        self.title("Pomodoro");
        self.section("Durations");
        let mins = |v: u32| format!("{v} min");
        let r = self.card(&[ROW, ROW, ROW, ROW]);
        self.stepper(c, r[0], "Focus", None, |c| &mut c.focus_min, 5, 5, 120, &mins);
        self.stepper(c, r[1], "Short break", None, |c| &mut c.short_break_min, 1, 1, 30, &mins);
        self.stepper(c, r[2], "Long break", None, |c| &mut c.long_break_min, 5, 5, 60, &mins);
        self.stepper(c, r[3], "Sessions before a long break", None, |c| &mut c.sessions_before_long, 1, 2, 8, &|v| v.to_string());
        self.section("Flow");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Start breaks automatically", Some("Begin the break as soon as focus ends"), |c| &mut c.auto_start_breaks);
        self.toggle(c, r[1], "Start focus automatically", Some("Begin the next session when a break ends"), |c| &mut c.auto_start_focus);
    }

    fn water(&mut self, c: &Config) {
        self.title("Water");
        self.section("Reminders");
        let hour = |v: u32| format!("{v:02}:00");
        let r = self.card(&[ROW_SUB, ROW, ROW, ROW]);
        self.toggle(c, r[0], "Water reminders", Some("A gentle nudge during your hours until the goal is met"), |c| &mut c.water_enabled);
        self.stepper(c, r[1], "Remind every", None, |c| &mut c.water_interval_min, 15, 15, 240, &|v| format!("{v} min"));
        self.stepper(c, r[2], "From", None, |c| &mut c.water_start_hour, 1, 0, 22, &hour);
        self.stepper(c, r[3], "Until", None, |c| &mut c.water_end_hour, 1, 1, 24, &hour);
        self.section("Amounts");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.stepper(c, r[0], "Glass size", Some("Added each time you press Drink"), |c| &mut c.glass_ml, 50, 50, 2000, &|v| format!("{v} ml"));
        self.stepper(c, r[1], "Daily goal", Some("Pick any goal that suits you"), |c| &mut c.goal_ml, 100, 500, 6000, &|v| format!("{:.1} L", v as f32 / 1000.0));
        self.section("Today");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], "Reset today's log", Some("Removes every drink logged today"), "Reset", Act::ResetWater, true);
    }

    fn shelf(&mut self, c: &Config, count: usize) {
        self.title("Shelf");
        self.section("Behaviour");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Open the shelf when dragging", Some("The notch opens as files move over it"), |c| &mut c.shelf_open_on_drag);
        self.toggle(c, r[1], "Remove after dragging out", Some("Otherwise items stay until you remove them"), |c| &mut c.shelf_remove_after_drag);
        self.toggle(c, r[2], "Show file sizes", None, |c| &mut c.shelf_show_sizes);
        self.section("Items");
        let r = self.card(&[ROW_SUB]);
        let sub = format!("{count} item{} on the shelf. Files themselves are not touched", if count == 1 { "" } else { "s" });
        self.button(r[0], "Clear the shelf", Some(&sub), "Clear all", Act::ClearShelf, true);
    }

    fn hud(&mut self, c: &Config) {
        self.title("HUD and popups");
        self.section("Show an overlay for");
        let r = self.card(&[ROW, ROW, ROW, ROW, ROW, ROW, ROW]);
        self.toggle(c, r[0], "Volume", None, |c| &mut c.hud_volume);
        self.toggle(c, r[1], "Brightness", None, |c| &mut c.hud_brightness);
        self.toggle(c, r[2], "Caps Lock", None, |c| &mut c.hud_caps);
        self.toggle(c, r[3], "Keyboard language", None, |c| &mut c.hud_language);
        self.toggle(c, r[4], "Battery and charger", None, |c| &mut c.hud_battery);
        self.toggle(c, r[5], "Wi-Fi", None, |c| &mut c.hud_wifi);
        self.toggle(c, r[6], "Headphones", None, |c| &mut c.hud_headphones);
        self.section("Style");
        let r = self.card(&[86.0, 86.0]);
        self.seg_row(c, r[0], "Layout", Some("Compact fits the notch, Wide adds a label"), &config::HUD_STYLE_LABELS, |c| &mut c.hud_style);
        self.seg_row(c, r[1], "Stays for", Some("How long an overlay is shown"), &config::HUD_DURATION_LABELS, |c| &mut c.hud_duration);
    }

    fn changelog(&mut self) {
        self.title("What's new");
        let version = env!("CARGO_PKG_VERSION");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], &format!("SuiNotch {version}"), Some("Release notes for the version you're running"), "Back to About", Act::Link(ABOUT_PAGE), false);
        let blocks = changelog::parse(changelog::TEXT);
        let mut bullets: Vec<&str> = Vec::new();
        for block in &blocks {
            if let changelog::Block::Bullet(b) = block {
                bullets.push(b);
                continue;
            }
            self.bullet_card(&bullets);
            bullets.clear();
            match block {
                changelog::Block::Heading(h) => self.section(h),
                changelog::Block::Text(t) => {
                    let gfx = self.cv.gfx;
                    for line in changelog::wrap(t, self.w - 8.0, |s| gfx.text_width(s, 13.0, 500)) {
                        self.cv.text(&line, R::new(self.x + 4.0, self.y, self.w - 8.0, NOTE_LINE), 13.0, 500, DIM, Align::Left);
                        self.y += NOTE_LINE;
                    }
                    self.y += 14.0;
                }
                changelog::Block::Bullet(_) => {}
            }
        }
        self.bullet_card(&bullets);
    }

    /// One card per run of bullets, one row per bullet, wrapped to the card width.
    fn bullet_card(&mut self, bullets: &[&str]) {
        if bullets.is_empty() {
            return;
        }
        const PAD: f32 = 16.0;
        const INDENT: f32 = 16.0;
        let text_w = self.w - PAD * 2.0 - INDENT;
        let gfx = self.cv.gfx;
        let wrapped: Vec<Vec<String>> = bullets.iter().map(|b| changelog::wrap(b, text_w, |s| gfx.text_width(s, 13.0, 500))).collect();
        let heights: Vec<f32> = wrapped.iter().map(|l| l.len() as f32 * NOTE_LINE + 20.0).collect();
        let rows = self.card(&heights);
        for (row, lines) in rows.iter().zip(&wrapped) {
            let top = row.y + 10.0;
            self.cv.fill_circle(row.x + PAD + 3.0, top + NOTE_LINE / 2.0, 2.5, DIM);
            for (i, line) in lines.iter().enumerate() {
                let lr = R::new(row.x + PAD + INDENT, top + i as f32 * NOTE_LINE, text_w, NOTE_LINE);
                self.cv.text(line, lr, 13.0, 500, C::WHITE, Align::Left);
            }
        }
    }

    fn about(&mut self, cfg: &Config, update: &Status) {
        self.title("About");
        let r = self.card(&[150.0]);
        let row = r[0];
        crate::tray::draw_mark_at(self.cv, row.x + 24.0, row.y + 30.0, 88.0);
        self.cv.display("SuiNotch", R::new(row.x + 130.0, row.y + 34.0, 300.0, 34.0), 26.0, 700, C::WHITE, Align::Left);
        self.cv.text(&format!("Version {}  ·  by {DEVELOPER}", env!("CARGO_PKG_VERSION")), R::new(row.x + 130.0, row.y + 70.0, 300.0, 18.0), 12.5, 500, DIM, Align::Left);
        self.cv.text("A living notch for Windows.", R::new(row.x + 130.0, row.y + 92.0, row.w - 150.0, 18.0), 12.5, 500, DIM, Align::Left);
        self.section("Developer");
        let r = self.card(&[ROW, ROW_SUB]);
        self.label(r[0], "Made by", None);
        self.cv.text(DEVELOPER, R::new(r[0].right() - 216.0, r[0].y, 200.0, r[0].h), 13.0, 600, C::WHITE, Align::Right);
        self.button(r[1], "Support", Some(SUPPORT_EMAIL), "Email us", Act::Support, false);
        self.section("Updates");
        let r = self.card(&[ROW_SUB, ROW_SUB, 86.0]);
        let current = env!("CARGO_PKG_VERSION");
        self.button(r[1], "What's new", Some(&format!("Changes in version {current}")), "View changelog", Act::Link(CHANGELOG_PAGE), false);
        let (label, sub, button) = match update {
            Status::Idle => ("Updates".to_string(), format!("Version {current}"), Some(("Check now", Act::CheckUpdate))),
            Status::Checking => ("Checking for updates…".to_string(), format!("Version {current}"), None),
            Status::UpToDate => ("You're up to date".to_string(), format!("Version {current} is the latest"), Some(("Check again", Act::CheckUpdate))),
            Status::Available(rel) => (format!("Version {} is available", rel.version), format!("You have {current}"), Some(("Install and restart", Act::InstallUpdate))),
            Status::Downloading(v) => (format!("Installing {v}…"), "SuiNotch restarts by itself in a moment".to_string(), None),
            Status::Failed(msg) => ("Couldn't update".to_string(), msg.clone(), Some(("Try again", Act::CheckUpdate))),
        };
        match button {
            Some((text, act)) => self.button(r[0], &label, Some(&sub), text, act, false),
            None => self.label(r[0], &label, Some(&sub)),
        }
        self.seg_row(cfg, r[2], "Automatic updates", Some("Automatic installs only while you're away from the PC"), &update::MODE_LABELS, |c| &mut c.update_mode);
        self.section("Your data");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], "Stored only on this PC", Some("Tasks, water log, shelf and settings. No accounts, no tracking"), "Open folder", Act::OpenData, false);
    }
}
