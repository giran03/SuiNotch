//! Native lyrics-mode interaction and full-notch layout check; isolated data, no network or playback.
use super::*;
use crate::lyric_style::LyricsMode;

#[test]
#[ignore = "native graphics; requires isolated APPDATA ending in lyrics-toggle-test"]
fn adaptive_volume_slider_renders_in_both_orientations() {
    let app = tasks::integration::fixture();
    let output = crate::config::data_dir();
    std::fs::create_dir_all(&output).unwrap();
    for scale in [1.0, 1.5, 2.0] {
        for (name, lyrics, room) in [("horizontal", false, 150.0), ("lyrics-open", true, 150.0), ("compact", false, 80.0)] {
            let (width, height) = ((200.0 * scale) as i32, (120.0 * scale) as i32);
            render::dc_snapshot(&app.gfx, &output.join(format!("volume-{name}-{scale}.png")), width, height, |dc| {
                let rt = gfx::dc_render_target(&app.gfx).unwrap();
                unsafe {
                    rt.BindDC(dc, &RECT { left: 0, top: 0, right: width, bottom: height }).unwrap();
                    rt.SetDpi(96.0 * scale, 96.0 * scale);
                    rt.BeginDraw();
                }
                let mut cv = Canvas::new(&app.gfx, rt.cast().unwrap()).unwrap();
                cv.clear(C::BLACK);
                crate::pages::draw_volume_flyout(&mut cv, views::volume_layout(160.0, 96.0, room, lyrics), 0.65, false, 1.0);
                cv.glyph(gfx::icon::VOLUME, 160.0, 96.0, 12.0, views::DIM);
                unsafe { rt.EndDraw(None, None).unwrap(); }
            }).unwrap();
        }
    }
    unsafe { DestroyWindow(app.notch.hwnd).unwrap(); }
}

#[test]
#[ignore = "native graphics; requires isolated APPDATA ending in lyrics-toggle-test"]
fn lyrics_button_cycles_outside_inside_both_off_and_renders_both_together() {
    assert!(crate::config::data_dir().parent().unwrap().ends_with("lyrics-toggle-test"));
    let mut app = tasks::integration::fixture();
    let output = crate::config::data_dir();
    std::fs::create_dir_all(&output).unwrap();
    let info = crate::events::MediaInfo { title: "A song for the lyrics preview".into(), artist: "SuiNotch".into(), app_id: "Spotify.exe".into(), playing: false, position_ms: 2500, duration_ms: 60000, sources: 1, ..Default::default() };
    app.media = Some(crate::app::Media { info, at: Instant::now() });
    crate::lyric_filter::reconcile(&mut app);
    loaded_result(&mut app, false);
    app.media.as_mut().unwrap().info.playing = true;
    let expected = (app.lyrics.clone(), app.lyrics_translation.clone(), app.lyrics_romaji.clone(), app.lyrics_words.clone(), app.lyrics_source, app.lyrics_key);
    app.cfg.lyrics_pill = false;
    app.cfg.lyrics_auto = true; // even auto-open must not override Off on restart
    let restarted = Notch::create(&app.cfg).unwrap();
    assert!(!restarted.lyrics_view);
    unsafe { DestroyWindow(restarted.hwnd).unwrap(); }
    app.notch.page = Page::Music;
    for (name, mode) in [("outside", LyricsMode::Outside), ("inside", LyricsMode::Inside), ("both", LyricsMode::Both), ("off", LyricsMode::Off)] {
        app.notch.expanded = true;
        views::click(&mut app, Hit::LyricsToggle, (false, false));
        assert_eq!(app.cfg.lyrics_mode(), mode);
        assert_eq!(app.notch.lyrics_view, matches!(mode, LyricsMode::Inside | LyricsMode::Both));
        let persisted: Config = crate::config::load_json("config.json");
        assert_eq!(persisted.lyrics_mode(), mode);
        assert_eq!((app.lyrics.clone(), app.lyrics_translation.clone(), app.lyrics_romaji.clone(), app.lyrics_words.clone(), app.lyrics_source, app.lyrics_key), expected);
        assert!(!app.lyric_request.searching);
        assert!(app.lyric_request.started);
        for scale in [1.0, 1.5, 2.0] {
            render::lyrics_snapshot(&mut app, &output.join(format!("lyrics-{name}-open-{scale}.png")), scale).unwrap();
            assert!(app.notch.hits.iter().any(|(_, hit)| *hit == Hit::LyricsToggle));
            app.notch.expanded = false;
            render::lyrics_snapshot(&mut app, &output.join(format!("lyrics-{name}-closed-{scale}.png")), scale).unwrap();
            assert_eq!(app.notch.pill_alpha.target > 0.5, mode != LyricsMode::Off);
            if mode == LyricsMode::Both {
                let look = app.cfg.lyric_look();
                assert!(look.in_notch() && look.below_notch());
                assert!(app.notch.win.1 > app.notch.h.value + SHADOW);
                assert!(app.notch.pill_inline_w > 0.0);
            }
            app.notch.expanded = true;
        }
    }
    // Notices use the outside slot in every lyrics mode, and the surface includes the whole pill.
    for (name, mode) in [("outside", LyricsMode::Outside), ("inside", LyricsMode::Inside), ("both", LyricsMode::Both), ("off", LyricsMode::Off)] {
        views::select_lyrics_mode(&mut app, mode);
        debug::notification(&mut app);
        for expanded in [false, true] {
            app.notch.expanded = expanded;
            for scale in [1.0, 1.5, 2.0] {
                render::lyrics_snapshot(&mut app, &output.join(format!("notice-{name}-{expanded}-{scale}.png")), scale).unwrap();
                let (width, height) = views::notice_size(&app).unwrap();
                assert!(app.notch.win.0 >= width);
                assert!(app.notch.win.1 >= app.notch.body().bottom() + 8.0 + height);
                assert_eq!(app.lyrics, expected.0, "notices retain lyrics for restoration");
            }
        }
        app.notch.notice.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(2);
        render::expire_transients(&mut app);
        assert!(app.notch.notice.is_none());
    }
    // Off -> Outside restores the same result without a replacement request.
    views::click(&mut app, Hit::LyricsToggle, (false, false));
    assert_eq!(app.cfg.lyrics_mode(), LyricsMode::Outside);
    assert_eq!((app.lyrics.clone(), app.lyrics_translation.clone(), app.lyrics_romaji.clone(), app.lyrics_words.clone(), app.lyrics_source, app.lyrics_key), expected);
    assert!(!app.lyric_request.searching);
    browser_manual_result_survives_toggles(&mut app);
    // Menu choices jump directly to any mode through the same persistence and loaded-result path.
    loaded_result(&mut app, true);
    let retained = app.lyrics.clone();
    for mode in [LyricsMode::Both, LyricsMode::Outside, LyricsMode::Off, LyricsMode::Inside] {
        views::select_lyrics_mode(&mut app, mode);
        assert_eq!(app.cfg.lyrics_mode(), mode);
        assert_eq!(crate::config::load_json::<Config>("config.json").lyrics_mode(), mode);
        assert!(!app.lyric_request.searching, "choosing a mode cannot restart a browser search");
        assert_eq!(app.lyrics, retained, "direct choices retain the loaded lyrics");
    }
    views::select_lyrics_mode(&mut app, LyricsMode::Outside);
    lyric_filter_cancels_source_changes_and_rejects_queued_results(&mut app);
    unsafe { DestroyWindow(app.notch.hwnd).unwrap(); }
}

// Keep native checks on the same UI thread and window fixture.
fn lyric_filter_cancels_source_changes_and_rejects_queued_results(app: &mut App) {
    use crate::{events::{Event, MediaInfo, MediaKind}, lyric_filter::{self, Policy}};
    assert!(crate::config::data_dir().parent().unwrap().ends_with("lyrics-toggle-test"));
    app.cfg.lyrics_pill = true;
    let song = MediaInfo { title: "Song".into(), artist: "Artist".into(), app_id: "Spotify.exe".into(), kind: MediaKind::Music, playing: true, duration_ms: 180000, ..Default::default() };
    reminders::on_event(app, Event::Media(Some(song.clone())));
    assert!(app.lyric_request.searching);
    let old_generation = app.lyric_request.generation;
    for _ in 0..2 { // Outside -> Inside -> Both must keep an in-flight automatic request too.
        views::click(app, Hit::LyricsToggle, (false, false));
        assert!(app.lyric_request.searching);
        assert_eq!(app.lyric_request.generation, old_generation);
    }
    let browser = MediaInfo { app_id: "chrome.exe".into(), ..song.clone() };
    reminders::on_event(app, Event::Media(Some(browser.clone())));
    assert!(!app.lyric_request.started);
    assert!(app.lyrics.is_empty());
    reminders::on_event(app, Event::Lyrics { key: song.track_key(), generation: old_generation, outcome: crate::lyrics::Outcome::NotFound });
    assert_eq!(app.lyrics_key, 0); // queued same-title result from a different source is discarded
    assert!(lyric_filter::manual_allowed(app));
    lyric_filter::clear(app);
    app.cfg.lyrics_sources.insert(browser.app_id.clone(), Policy::Never);
    lyric_filter::reconcile(app);
    assert!(!lyric_filter::manual_allowed(app));
    views::click(app, Hit::LyricsRetry, (false, false));
    assert!(!app.lyric_request.started);
    app.cfg.lyrics_sources.clear();
    reminders::on_event(app, Event::Media(Some(song.clone())));
    assert!(app.lyric_request.searching);
    reminders::on_event(app, Event::Media(Some(MediaInfo { playing: false, ..song.clone() })));
    assert!(!app.lyric_request.started);
    reminders::on_event(app, Event::Media(Some(song)));
    assert!(app.lyric_request.searching);
    let old = app.cfg.clone();
    app.cfg.lyrics_pill = false;
    config_changed(app, &old);
    assert!(!app.lyric_request.started);
    assert!(!lyric_filter::manual_allowed(app));
    reminders::on_event(app, Event::Media(None));
    assert_eq!(app.lyric_request.generation, 0);
}

fn loaded_result(app: &mut App, manual: bool) {
    app.lyric_request.started = true;
    app.lyric_request.manual = manual;
    app.lyric_request.generation = u64::MAX; // synthetic completed request, never a background worker
    let lines = vec![crate::lrc::Line { at_ms: 0, text: "The same lyric, inside and outside".into() }, crate::lrc::Line { at_ms: 15000, text: "And the next line follows the song".into() }];
    reminders::on_event(app, crate::events::Event::Lyrics {
        key: app.media.as_ref().unwrap().info.track_key(), generation: u64::MAX,
        outcome: crate::lyrics::Outcome::Found(crate::lyrics::Found { lines: lines.clone(), source: crate::lyrics::Source::Lrclib, synced: true,
            translation: lines.clone(), romaji: lines,
            words: vec![vec![crate::words::Word { start_ms: 0, dur_ms: 15000, chars: 39 }]],
            id: "lrclib:1".into(),
        }),
    });
}

fn browser_manual_result_survives_toggles(app: &mut App) {
    use crate::{events::{Event, MediaInfo, MediaKind}, lyric_filter::{self, Policy}};
    let browser = MediaInfo { title: "Browser song".into(), artist: "Artist".into(), app_id: "msedge.exe".into(), kind: MediaKind::Music, playing: true, ..Default::default() };
    app.cfg.lyrics_sources.insert(browser.app_id.clone(), Policy::Automatic); // legacy opt-in must not bypass the browser gate
    reminders::on_event(app, Event::Media(Some(browser.clone())));
    assert!(!app.lyric_request.started);
    assert!(lyric_filter::manual_allowed(app));
    assert!(app.lyrics.is_empty());
    loaded_result(app, true);
    let expected = (app.lyrics.clone(), app.lyrics_translation.clone(), app.lyrics_romaji.clone(), app.lyrics_words.clone(), app.lyrics_source, app.lyrics_key);
    for _ in 0..8 {
        let generation = app.lyric_request.generation;
        views::click(app, Hit::LyricsToggle, (false, false));
        reminders::on_event(app, Event::Media(Some(browser.clone()))); // player updates while Off must not discard the retained result
        assert_eq!((app.lyrics.clone(), app.lyrics_translation.clone(), app.lyrics_romaji.clone(), app.lyrics_words.clone(), app.lyrics_source, app.lyrics_key), expected);
        assert!(app.lyric_request.manual);
        assert!(!app.lyric_request.searching);
        assert_eq!(app.lyric_request.generation, generation);
    }
    app.cfg.lyrics_pill = true;
    lyric_filter::request(app, true, true); // changing translation/word style must not trigger browser requests
    assert!(!app.lyric_request.searching);
    assert_eq!(app.lyrics, expected.0);
    reminders::on_event(app, Event::Media(Some(MediaInfo { title: "Different video".into(), ..browser })));
    assert!(app.lyrics.is_empty());
    assert!(!app.lyric_request.manual);
    assert!(!app.lyric_request.started);
}
