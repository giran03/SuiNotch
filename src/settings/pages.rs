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
            _ if !self.s.query.is_empty() => self.search_results(),
            0 => self.general(&cfg),
            1 => self.display(&cfg),
            2 => self.appearance(&cfg),
            3 => {
                // What the song on screen has, so the Second line setting can say whether it will show anything.
                let extras = (!app.lyrics.is_empty()).then(|| (!app.lyrics_translation.is_empty(), !app.lyrics_romaji.is_empty()));
                self.music(&cfg, app.recent.items.len(), extras)
            }
            4 => self.todo(&cfg),
            5 => self.calendar(&cfg, app),
            6 => self.pomodoro(&cfg),
            7 => self.water(&cfg),
            8 => self.shelf(&cfg, app.shelf.items.len()),
            9 => self.clipboard(&cfg, app.clips.items.len()),
            10 => self.hud(&cfg),
            CHANGELOG_PAGE => self.changelog(),
            DIAG_PAGE => self.diagnostics(),
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
        self.section("Pages in the notch");
        let subs = [
            "Now playing, controls and lyrics",
            "Pomodoro, timer and stopwatch",
            "Today's water and the hourly chart",
            "Files and text you drop on the notch",
            "Recent copies, while clipboard history is on",
            "Your tasks and reminders",
            "Your month, tasks and calendar events",
            "This week's focus, water and tasks, with streaks",
        ];
        let r = self.card(&[ROW_SUB; 8]);
        for (i, sub) in subs.iter().enumerate() {
            self.page_toggle(c, r[i], i, sub);
        }
        self.cv.text("Hidden pages keep working: reminders, timers and clipboard history carry on.", R::new(self.x + 4.0, self.y - 12.0, self.w, 18.0), 11.5, 500, DIM, Align::Left);
        self.y += 14.0;
        self.section("Notch");
        let r = self.card(&[ROW_SEG, ROW_SUB, ROW_SUB, ROW_SUB, ROW_SUB]);
        self.seg_row(c, r[0], "Size", Some("Applies right away. Auto is larger on wide monitors"), &config::SIZE_LABELS, |c| &mut c.size);
        self.toggle(c, r[1], "Minimal notch", Some("A thin line when nothing is playing"), |c| &mut c.minimal);
        self.toggle(c, r[2], "Tooltips", Some("A short hint when you rest on a button that needs explaining"), |c| &mut c.tooltips);
        self.toggle(c, r[3], "Sui mascot", Some("A little droplet that lives in the notch"), |c| &mut c.mascot);
        self.toggle(c, r[4], "Sui reactions", Some("Sui cheers for finished focus sessions, goals and tasks, and naps on long breaks"), |c| &mut c.sui_reactions);
        self.section("Sounds and quiet");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Reminder sounds", Some("Chimes for Pomodoro, To-do and Water"), |c| &mut c.sounds);
        self.toggle(c, r[1], "Respect Do Not Disturb", Some("Hold reminders while Windows is in Do Not Disturb, presenting or gaming"), |c| &mut c.respect_quiet);
        self.section("Full screen");
        let r = self.card(&[ROW_SUB]);
        self.toggle(c, r[0], "Hide in full screen", Some("Games, videos and slides that fill the screen"), |c| &mut c.hide_fullscreen);
    }

    fn display(&mut self, c: &Config) {
        self.title("Display");
        self.section("Show the notch on");
        let monitors = self.s.monitors.clone();
        let mut heights = vec![ROW, ROW_SUB, ROW_SUB];
        heights.extend(std::iter::repeat_n(ROW, monitors.len()));
        let rows = self.card(&heights);
        let accent = Self::accent(c);
        let (follow, follow_mouse) = (c.display_follow, c.display_follow_mouse);
        let fixed = !follow && !follow_mouse;
        let auto = fixed && (c.display.is_none() || !monitors.iter().any(|m| Some(&m.device) == c.display.as_ref()));
        self.radio(rows[0], "Main display", Some("Follows the primary monitor"), auto, Act::Display(None), accent);
        self.radio(rows[1], "Follow the active window", Some("The notch moves to the screen you're working on"), follow, Act::DisplayFollow, accent);
        self.radio(rows[2], "Follow the mouse", Some("The notch moves to the screen the pointer rests on"), follow_mouse, Act::DisplayFollowMouse, accent);
        for (i, m) in monitors.iter().enumerate() {
            let size = format!("{} x {}", m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
            let name = format!("Display {}{}", i + 1, if m.primary { " (primary)" } else { "" });
            let selected = fixed && c.display.as_deref() == Some(m.device.as_str());
            self.radio_with_meta(rows[i + 3], &name, &size, selected, Act::Display(Some(i)), accent);
        }
        self.section("Behaviour");
        let r = self.card(&[ROW_SUB, ROW_SEG]);
        self.toggle(c, r[0], "Open on hover", Some("Otherwise click the notch to open it"), |c| &mut c.expand_on_hover);
        self.seg_row(c, r[1], "Hover delay", Some("How long to rest on the notch before it opens"), &config::HOVER_DELAY_LABELS, |c| &mut c.hover_delay);
    }

    fn radio(&mut self, r: R, label: &str, sub: Option<&str>, on: bool, act: Act, accent: C) {
        self.s.hits.push((r, act));
        if hovered(self.s, act) {
            self.cv.fill_rrect(r.inset(4.0, 3.0), 9.0 * self.round, C::WHITE.a(0.03));
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
        let r = self.card(&[64.0, ROW_SUB]);
        let row = r[0];
        let follow = c.accent_follow_system;
        for (i, rgb) in ACCENTS.iter().enumerate() {
            let cx = row.x + 32.0 + i as f32 * 46.0;
            let act = Act::Accent(i);
            self.s.hits.push((R::centered(cx, row.cy(), 40.0, 40.0), act));
            // While Windows picks the colour, our swatches stay available but step back.
            let col = C::hex(*rgb).a(if follow { 0.4 } else { 1.0 });
            let grow = if hovered(self.s, act) { 1.5 } else { 0.0 };
            self.cv.fill_circle(cx, row.cy(), 13.0 + grow, col);
            if c.accent == i && !follow {
                self.cv.stroke_circle(cx, row.cy(), 18.0, col, 2.0);
            }
        }
        let sub = match sys::system_accent() {
            Some(_) => "Matches the colour in Windows Settings > Personalization > Colors",
            None => "Windows' accent colour couldn't be read, so your pick above is used",
        };
        self.toggle(c, r[1], "Use the Windows accent colour", Some(sub), |c| &mut c.accent_follow_system);
        self.section("Style");
        let r = self.card(&[ROW_SEG, ROW_SUB]);
        self.seg_row(c, r[0], "Notch style", Some("Docked blends into the screen edge, Floating hovers below it, Flat sits square against it"), &config::NOTCH_STYLE_LABELS, |c| &mut c.notch_style);
        self.stepper(c, r[1], "Floating gap", Some("Space between the screen edge and a floating notch"), |c| &mut c.notch_gap, 2, config::MIN_NOTCH_GAP, config::MAX_NOTCH_GAP, &|v| format!("{v} px"));
        self.section("Shape");
        let r = self.card(&[ROW_SEG, ROW_SUB, ROW_SUB]);
        self.seg_row(c, r[0], "Corner roundness", Some("How soft the bottom corners of the notch are"), &config::ROUNDNESS_LABELS, |c| &mut c.roundness);
        self.toggle(c, r[1], "Frosted glass", Some("A slightly translucent notch with a fine edge"), |c| &mut c.glass);
        self.toggle(c, r[2], "Shadow", Some("A soft shadow under the open notch"), |c| &mut c.shadow);
        self.section("Motion");
        let r = self.card(&[ROW_SEG]);
        self.seg_row(c, r[0], "Animation style", Some("Spring feel for opening, closing and switching pages"), &config::ANIMATION_LABELS, |c| &mut c.animation);
        self.section("Clock");
        let r = self.card(&[ROW_SEG]);
        self.seg_row(c, r[0], "Time format", Some("System follows your Windows region settings"), &config::CLOCK_LABELS, |c| &mut c.clock_format);
    }

    /// `extras` is (has a translation, has romaji) for the song on screen, or None when it has no lyrics.
    fn music(&mut self, c: &Config, recent: usize, extras: Option<(bool, bool)>) {
        self.title("Music and lyrics");
        self.section("Lyrics");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Open lyrics automatically", Some("Show the lyrics panel whenever the Music page opens"), |c| &mut c.lyrics_auto);
        self.toggle(c, r[1], "Show lyrics in the closed notch", Some("The current line, in the look chosen below"), |c| &mut c.lyrics_pill);
        let tries = |v: u32| match v {
            0 => "No retries".to_string(),
            1 => "1 retry".to_string(),
            n => format!("{n} retries"),
        };
        self.stepper(c, r[2], "Keep searching for", Some("When no lyrics are found, search again this many times. Each retry tries the English and native titles"), |c| &mut c.lyrics_retries, 1, 0, config::MAX_LYRICS_RETRIES, &tries);
        self.lyric_look_settings(c, extras);
        self.section("Now playing");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Equalizer animation", Some("Moving bars in the closed notch while music plays"), |c| &mut c.equalizer);
        self.toggle(c, r[1], "Artwork glow", Some("Tint the notch with the colours of the album cover"), |c| &mut c.artwork_glow);
        self.toggle(c, r[2], "Show player name", Some("Spotify, Apple Music, Chrome and others"), |c| &mut c.show_source);
        self.section("Privacy");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB, ROW_SUB, ROW_SUB]);
        self.button(r[0], "Lyrics come from LRCLIB", Some("Only the song title, artist, album and length are sent"), "lrclib.net", Act::Website, false);
        self.toggle(c, r[1], "Fall back to NetEase", Some("Asks NetEase when LRCLIB has none or for romaji and translations, and Apple's store for native titles. Sends the title and artist"), |c| &mut c.lyrics_fallback);
        self.toggle(c, r[2], "Keep lyrics on this PC", Some("A song already looked up shows at once, even offline"), |c| &mut c.lyrics_cache);
        self.button(r[3], "Clear saved lyrics", Some("Removes the lyrics kept on this PC; they are fetched again when needed"), "Clear", Act::ClearLyricsCache, true);
        let sub = format!("{recent} song{} in the Recently played list", if recent == 1 { "" } else { "s" });
        self.button(r[4], "Clear recently played", Some(&sub), "Clear", Act::ClearRecent, true);
    }

    /// The lyric look: a live preview, then each choice. Rows the other choices make pointless are dimmed.
    fn lyric_look_settings(&mut self, c: &Config, extras: Option<(bool, bool)>) {
        use crate::lyric_style as ls;
        let off = !c.lyrics_pill;
        let in_notch = !c.lyric_look().below_notch();
        self.section("Lyrics look");
        let r = self.card(&[ROW_PREVIEW, ROW_SEG, ROW_SEG, ROW_SEG, ROW_SEG, ROW_SEG]);
        self.lyric_preview(c, r[0]);
        self.dimmed(off, |ui| {
            ui.seg_row(c, r[1], "Lyric preset", Some("Start from a look, then change any choice below; it reads Custom once they match no preset"), &ls::PRESET_LABELS, |c| &mut c.lyrics_preset);
            ui.seg_row(c, r[2], "Placement", Some("Under the notch, or in it in place of the song title"), &ls::PLACE_LABELS, |c| &mut c.lyrics_place);
        });
        self.dimmed(off || in_notch, |ui| {
            let sub = if in_notch { "Not used in the notch: the notch itself is the backdrop" } else { "What sits behind the line. Album tint follows the cover" };
            ui.seg_row(c, r[3], "Backdrop", Some(sub), &ls::BACKDROP_LABELS, |c| &mut c.lyrics_backdrop);
        });
        self.dimmed(off, |ui| {
            ui.seg_row(c, r[4], "Highlight", Some("Karaoke fill colours the line as it is sung, wherever it is placed"), &ls::HIGHLIGHT_LABELS, |c| &mut c.lyrics_highlight);
            ui.seg_row(c, r[5], "Text colour", Some("Album tint follows the cover; Accent follows your accent colour"), &ls::COLOR_LABELS, |c| &mut c.lyrics_color);
        });
        self.section("Lyrics text");
        let r = self.card(&[ROW_SEG, ROW_SEG, ROW_SEG, ROW_SEG]);
        self.dimmed(off, |ui| {
            ui.seg_row(c, r[0], "Text size", Some("The size of the line; a second line is a little smaller"), &ls::SIZE_LABELS, |c| &mut c.lyrics_size);
            ui.seg_row(c, r[1], "New lines", Some("How each new line arrives"), &ls::ENTER_LABELS, |c| &mut c.lyrics_enter);
            ui.seg_row(c, r[2], "Long lines", Some("A line wider than there is room for can shrink, scroll to its end, or be cut"), &ls::FIT_LABELS, |c| &mut c.lyrics_fit);
            ui.seg_row(c, r[3], "When paused", Some("Keep the line on show, dimmed, while the music is paused"), &ls::PAUSED_LABELS, |c| &mut c.lyrics_paused);
        });
        self.section("Second line");
        let r = self.card(&[ROW_SEG, ROW_SEG]);
        // Says whether the song on screen has what is asked for, so an empty second line is not a mystery.
        let yes = |b: bool| if b { "yes" } else { "no" };
        let note = match extras {
            Some((translation, romaji)) => format!("Under the line, or after it in the notch. This song: translation {}, romaji {}", yes(translation), yes(romaji)),
            None => "Under the line, or after it in the notch. Romaji and translations come from NetEase, for songs that have them".to_string(),
        };
        self.dimmed(off, |ui| ui.seg_row(c, r[0], "Second line", Some(&note), &ls::SECOND_LABELS, |c| &mut c.lyrics_second));
        self.seg_row(c, r[1], "Under each line in the panel", Some("Romaji or a translation in the open lyrics panel; its Aあ button switches it too"), &ls::PANEL_ALT_LABELS, |c| &mut c.lyrics_panel_alt);
    }

    fn todo(&mut self, c: &Config) {
        self.title("To-do");
        self.section("Reminders");
        let r = self.card(&[ROW_SEG]);
        self.seg_row(c, r[0], "Snooze for", Some("When a \"Time's up!\" reminder is snoozed"), &config::SNOOZE_LABELS, |c| &mut c.todo_snooze);
        self.section("List");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Show completed tasks", Some("Keep finished tasks visible, crossed out"), |c| &mut c.todo_show_done);
        self.toggle(c, r[1], "Clear completed tasks daily", Some("Finished tasks disappear the next morning"), |c| &mut c.todo_clear_done_daily);
    }

    fn calendar(&mut self, c: &Config, app: &App) {
        self.title("Calendar");
        self.section("Calendar page");
        let r = self.card(&[ROW_SEG, ROW_SUB]);
        self.label(R::new(r[0].x, r[0].y, r[0].w, 38.0).offset(0.0, 4.0), "Week starts on", None);
        let bar = R::new(r[0].x + 16.0, r[0].bottom() - 42.0, r[0].w - 32.0, 30.0);
        self.week_seg(c, bar);
        self.toggle(c, r[1], "Day dots", Some("Mark days that have tasks or events"), |c| &mut c.calendar_dots);
        self.calendar_link(c, app);
    }

    /// The calendar link: paste one, then see its status, refresh it or remove it.
    fn calendar_link(&mut self, c: &Config, app: &App) {
        self.section("Events from your calendar");
        if c.calendar_url.is_empty() {
            let r = self.card(&[ROW_SUB, ROW_SUB]);
            let sub = if app.calendar_error.is_empty() { "Copy your calendar's iCal address, then press Paste link" } else { app.calendar_error.as_str() };
            self.button(r[0], "Add a calendar link", Some(sub), "Paste link", Act::PasteCalendar, false);
            self.label(r[1], "Where to find it", Some("Outlook: Publish a calendar, ICS link.  Google: Secret address in iCal format"));
            return;
        }
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB]);
        let status = if !app.calendar_error.is_empty() {
            app.calendar_error.clone()
        } else {
            match app.calendar_synced {
                Some(t) => format!("{} events  ·  updated {}", app.calendar.len(), sys::format_time(&crate::store::local(t), c.clock_format)),
                None => "Loading events…".to_string(),
            }
        };
        self.button(r[0], &crate::feed::host(&c.calendar_url), Some(&status), "Refresh", Act::RefreshCalendar, false);
        self.toggle(c, r[1], "Heads-up before events", Some("A reminder in the notch 10 minutes before each event"), |c| &mut c.calendar_reminders);
        self.button(r[2], "Remove this calendar", Some("Its events leave SuiNotch; the calendar itself isn't touched"), "Remove", Act::RemoveCalendar, true);
    }

    fn week_seg(&mut self, c: &Config, bar: R) {
        let monday = c.week_starts_monday;
        self.cv.fill_rrect(bar, 8.0 * self.round, C::WHITE.a(0.06));
        let half = bar.w / 2.0;
        let sel = if monday { 0 } else { 1 };
        self.cv.fill_rrect(R::new(bar.x + 2.0 + sel as f32 * half, bar.y + 2.0, half - 4.0, bar.h - 4.0), 6.5 * self.round, C::WHITE.a(0.16));
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

    fn clipboard(&mut self, c: &Config, count: usize) {
        self.title("Clipboard");
        self.section("History");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SEG]);
        self.toggle(c, r[0], "Keep clipboard history", Some("Texts and images you copy, on the Clipboard page"), |c| &mut c.clipboard_history);
        let (lo, hi) = (crate::clips::MIN_LIMIT, crate::clips::MAX_LIMIT);
        self.stepper(c, r[1], "Items to keep", Some("Older copies drop off as new ones arrive"), |c| &mut c.clipboard_limit, 5, lo, hi, &|v| format!("{v} items"));
        self.seg_row(c, r[2], "Keep after a restart", Some("History comes back if SuiNotch was off for less than this"), &crate::clips::KEEP_LABELS, |c| &mut c.clipboard_keep);
        self.section("When you copy");
        let r = self.card(&[ROW_SUB]);
        self.toggle(c, r[0], "Show a popup", Some("A short note in the notch with what you copied"), |c| &mut c.hud_clipboard);
        self.section("Privacy");
        let r = self.card(&[ROW_SUB]);
        let sub = format!("{count} item{} kept now. Password managers' copies are never kept", if count == 1 { "" } else { "s" });
        self.button(r[0], "Clear clipboard history", Some(&sub), "Clear", Act::ClearClips, true);
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
        let r = self.card(&[ROW_SEG, ROW_SEG]);
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
        let r = self.card(&[ROW, ROW_SUB, ROW_SUB]);
        self.label(r[0], "Made by", None);
        self.cv.text(DEVELOPER, R::new(r[0].right() - 216.0, r[0].y, 200.0, r[0].h), 13.0, 600, C::WHITE, Align::Right);
        self.button(r[1], "Support", Some(SUPPORT_EMAIL), "Email us", Act::Support, false);
        self.button(r[2], "Support the developers", Some("SuiNotch is free. A coffee helps keep it growing"), "Buy us a coffee", Act::Coffee, false);
        self.section("Updates");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SEG]);
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
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.button(r[0], "Stored only on this PC", Some("Tasks, water log, shelf and settings. No accounts, no tracking"), "Open folder", Act::OpenData, false);
        self.button(r[1], "Diagnostics", Some("Monitors, full-screen detection, players and the log"), "Open", Act::Link(DIAG_PAGE), false);
    }

    /// Results for what is typed in the sidebar search; each opens the page its setting lives on.
    fn search_results(&mut self) {
        const MAX_RESULTS: usize = 12;
        let query = self.s.query.clone();
        self.title("Search");
        let found = search::matches(&query);
        if found.is_empty() {
            self.cv.text(&format!("No settings match \"{query}\""), R::new(self.x + 4.0, self.y, self.w - 8.0, 22.0), 13.0, 500, DIM, Align::Left);
            self.y += 40.0;
            return;
        }
        self.section(&format!("{} result{}", found.len(), if found.len() == 1 { "" } else { "s" }));
        let shown: Vec<usize> = found.into_iter().take(MAX_RESULTS).collect();
        let rows = self.card(&vec![ROW_SUB; shown.len()]);
        for (row, &i) in rows.iter().zip(&shown) {
            let entry = &search::INDEX[i];
            let act = Act::Found(i);
            self.s.hits.push((*row, act));
            if hovered(self.s, act) {
                self.cv.fill_rrect(row.inset(4.0, 3.0), 9.0 * self.round, C::WHITE.a(0.03));
            }
            self.label_fit(*row, entry.label, Some(search::page_name(entry.page)), 44.0);
            self.cv.glyph('\u{E76C}', row.right() - 26.0, row.cy(), 10.0, DIM);
        }
    }

    /// What SuiNotch sees on this PC, for troubleshooting; see `diag` for how it is gathered.
    fn diagnostics(&mut self) {
        self.title("Diagnostics");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], "Taken when this page opened", Some("Refresh to look again, for example after a game starts"), "Refresh", Act::DiagRefresh, false);
        let diag = self.s.diag.clone();
        for (name, rows) in &diag.groups {
            self.section(name);
            let cards = self.card(&vec![ROW; rows.len()]);
            for (row, (key, value)) in cards.iter().zip(rows) {
                self.label_fit(*row, key, None, row.w * 0.62);
                self.cv.text(value, R::new(row.x + row.w * 0.38, row.y, row.w * 0.62 - 16.0, row.h), 12.0, 500, DIM, Align::Right);
            }
        }
        self.section("Log");
        if diag.log.is_empty() {
            self.cv.text("Nothing has been logged", R::new(self.x + 4.0, self.y, self.w - 8.0, 20.0), 12.5, 500, DIM, Align::Left);
            self.y += 34.0;
        } else {
            let cards = self.card(&vec![22.0; diag.log.len()]);
            for (row, line) in cards.iter().zip(&diag.log) {
                self.cv.text(line, R::new(row.x + 16.0, row.y, row.w - 32.0, row.h), 11.0, 500, DIM, Align::Left);
            }
        }
        self.section("Tools");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB]);
        self.button(r[0], "Copy debug info", Some("Everything above, ready to paste into a bug report"), "Copy", Act::DiagCopy, false);
        self.button(r[1], "Open the log file", Some("suinotch.log in the data folder"), "Open", Act::OpenLog, false);
        self.button(r[2], "Back to About", None, "Back", Act::Link(ABOUT_PAGE), false);
    }
}
