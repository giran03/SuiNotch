//! Opt-in native rendering and interaction check; never touches real user data or desktop input.
use super::*;
use crate::notch::Notch;
use std::rc::Rc;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;

fn fixture() -> App {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    assert!(config::data_dir().parent().unwrap().ends_with("sound-test"));
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
    }
    let cfg = Config { start_with_windows: false, sounds: false, respect_quiet: false, water_enabled: false, ..Config::default() };
    let durations = app::durations(&cfg);
    let hwnd = super::super::create_window().unwrap(); // hidden; no message loop or input injection
    let settings = SettingsWindow {
        hwnd,
        rt: None,
        page: SOUNDS_PAGE,
        scroll: Spring::new(0.0, 0.32, 1.0),
        content_h: 0.0,
        hits: Vec::new(),
        hover: None,
        pressed: None,
        knobs: HashMap::new(),
        segs: HashMap::new(),
        side_sel: Spring::new(SOUNDS_PAGE as f32, 0.3, 0.86),
        monitors: Vec::new(),
        query: String::new(),
        diag: diag::Diag::default(),
        confirm_reset: false,
        sound: State::default(),
        corners: usize::MAX,
        mouse: (-1.0, -1.0),
        last: Instant::now(),
        scale: 1.0,
    };
    App {
        gfx: Rc::new(gfx::Gfx::new().unwrap()),
        notch: Notch::create(&cfg).unwrap(),
        todos: Default::default(),
        water: Default::default(),
        shelf: Default::default(),
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
        recent: Default::default(),
        history: Default::default(),
        lyric_request: Default::default(),
        lyrics_attempt: 0,
        shifts: Default::default(),
        pomo: crate::pomodoro::Pomodoro::new(&durations),
        timer: crate::timers::Countdown::new(cfg.timer_min),
        stopwatch: Default::default(),
        stats: Default::default(),
        clips: crate::clips::Clips::new(10),
        calendar: Vec::new(),
        calendar_error: String::new(),
        calendar_synced: None,
        calendar_alerted: Default::default(),
        settings: Some(settings),
        events: crate::events::init(),
        started: Instant::now(),
        network: None,
        language: String::new(),
        caps: false,
        last_battery: None,
        last_day: crate::store::day_of(crate::store::now()),
        update: crate::update::Status::Idle,
        cfg,
    }
}

fn snapshot(app: &mut App, name: &str, scale: f32) {
    let mut s = app.settings.take().unwrap();
    let gfx = app.gfx.clone();
    let (width, height) = ((W * scale) as u32, (H * scale) as u32);
    unsafe {
        let bitmap = gfx.wic.CreateBitmap(width, height, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad).unwrap();
        let properties = D2D1_RENDER_TARGET_PROPERTIES { dpiX: 96.0 * scale, dpiY: 96.0 * scale, ..Default::default() };
        let rt = gfx.d2d.CreateWicBitmapRenderTarget(&bitmap, &properties).unwrap();
        rt.BeginDraw();
        let mut cv = Canvas::new(&gfx, rt.clone()).unwrap();
        cv.clear(BG);
        s.hits.clear();
        super::super::draw_sidebar(&cv, &mut s, 1.0);
        let mut ui = Ui { cv: &mut cv, s: &mut s, y: 0.0, x: SIDEBAR + 28.0, w: W - SIDEBAR - 56.0, row_index: 0, moving: false, dt: 0.016, round: 1.0 };
        ui.page(app);
        super::super::draw_title_buttons(&cv, &mut s);
        rt.EndDraw(None, None).unwrap();
        let path = config::data_dir().join(format!("{name}-{scale}.png"));
        let stream = gfx.wic.CreateStream().unwrap();
        stream.InitializeFromFilename(&windows::core::HSTRING::from(path.as_os_str()), 0x40000000).unwrap();
        let encoder = gfx.wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).unwrap();
        encoder.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()).unwrap();
        let frame = frame.unwrap();
        frame.Initialize(None).unwrap();
        frame.SetSize(width, height).unwrap();
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format).unwrap();
        frame.WriteSource(&bitmap, std::ptr::null()).unwrap();
        frame.Commit().unwrap();
        encoder.Commit().unwrap();
    }
    app.settings = Some(s);
}
fn scroll_to(app: &mut App, action: Action) {
    let s = app.settings.as_mut().unwrap();
    let rect = s.hits.iter().find(|(_, a)| *a == Act::Sound(action)).unwrap().0;
    s.scroll.snap((s.scroll.value + rect.y - 110.0).max(0.0));
}

#[test]
#[ignore = "native graphics; requires isolated APPDATA ending in sound-test"]
fn sound_library_native_flow() {
    let mut app = fixture();
    std::fs::create_dir_all(config::data_dir()).unwrap();
    for scale in [1.0, 1.5, 2.0] {
        snapshot(&mut app, "sounds", scale);
    }
    act(&mut app, Action::Pick(0));
    let choice = app.cfg.sound_library.choices().iter().position(|(id, _)| id == "glass").unwrap();
    act(&mut app, Action::Assign(0, choice));
    assert_eq!(app.cfg.sound_library.assignments[0], "glass");
    let source = config::data_dir().join("import-source.wav");
    std::fs::write(&source, include_bytes!("../sound/fixtures/tone.wav")).unwrap();
    begin_import(&mut app, source.clone(), None);
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while IMPORTING.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "import timed out");
        if let Ok(crate::events::Event::SoundImported { result, replacement }) = app.events.recv_timeout(std::time::Duration::from_millis(50)) {
            import_finished(&mut app, result, replacement);
        }
    }
    std::fs::remove_file(source).unwrap();
    assert_eq!(app.cfg.sound_library.custom.len(), 1);
    let id = app.cfg.sound_library.custom[0].id.clone();
    assert!(sound::available(&id));
    act(&mut app, Action::Edit(0));
    act(&mut app, Action::Rename(0));
    for _ in 0..60 {
        character(&mut app, 8);
    }
    for ch in "My chime".chars() {
        character(&mut app, ch as u32);
    }
    key(&mut app, VK_RETURN.0);
    assert_eq!(app.cfg.sound_library.custom[0].name, "My chime");
    act(&mut app, Action::Trim(0, true, 1000));
    act(&mut app, Action::Normalize(0));
    let custom = &app.cfg.sound_library.custom[0];
    assert!(custom.start_ms + custom.length_ms <= custom.duration_ms);
    assert!(custom.normalize);
    snapshot(&mut app, "before-scroll", 1.0);
    scroll_to(&mut app, Action::Edit(0));
    for scale in [1.0, 1.5, 2.0] {
        snapshot(&mut app, "custom-editor", scale);
    }
    let choice = app.cfg.sound_library.choices().iter().position(|(key, _)| *key == id).unwrap();
    act(&mut app, Action::Assign(3, choice));
    act(&mut app, Action::Defaults);
    assert_eq!(app.cfg.sound_library.assignments, sound::DEFAULTS.map(str::to_owned));
    assert_eq!(app.cfg.sound_library.custom.len(), 1);
    act(&mut app, Action::Assign(3, choice));
    // Replace via the same asynchronous path while keeping the assignment and display name.
    let replacement =
        sound::CustomSound { id: "custom-abcdef".into(), name: "replacement".into(), duration_ms: 1000, start_ms: 0, length_ms: 1000, normalize: false };
    import_finished(&mut app, Ok(replacement), Some(id));
    assert_eq!(app.cfg.sound_library.assignments[3], "custom-abcdef");
    assert_eq!(app.cfg.sound_library.custom[0].name, "My chime");
    act(&mut app, Action::Delete(0));
    assert_eq!(app.cfg.sound_library.custom.len(), 1); // first press only confirms
    act(&mut app, Action::Delete(0));
    assert!(app.cfg.sound_library.custom.is_empty());
    assert_eq!(app.cfg.sound_library.assignments[3], sound::DEFAULTS[3]);
    let restored: Config = config::load_json("config.json");
    assert!(restored.sound_library.custom.is_empty());
    assert_eq!(restored.sound_library.assignments, app.cfg.sound_library.assignments);
    // Alert dismissal and mute never leave playback running.
    crate::notch::show_alert(&mut app, crate::notch::AlertKind::Timer);
    assert!(!sound::ringing());
    crate::views::click(&mut app, crate::notch::Hit::AlertClose, (false, false));
    assert!(app.notch.alert.is_none());
    unsafe {
        DestroyWindow(app.settings.take().unwrap().hwnd).unwrap();
        DestroyWindow(app.notch.hwnd).unwrap();
    }
}

#[test]
#[ignore = "native graphics; requires isolated APPDATA ending in sound-test"]
fn lyric_source_preferences_render_and_persist() {
    let mut app = fixture();
    std::fs::create_dir_all(config::data_dir()).unwrap();
    let info = crate::events::MediaInfo { title: "Video title".into(), artist: "Creator".into(), app_id: "mpv.exe".into(), ..Default::default() };
    app.media = Some(crate::app::Media { info, at: Instant::now() });
    app.settings.as_mut().unwrap().page = 3;
    app.settings.as_mut().unwrap().side_sel.snap(3.0);
    snapshot(&mut app, "lyric-searches", 1.0);
    let s = app.settings.as_mut().unwrap();
    let row = s.hits.iter().find(|(_, a)| *a == Act::LyricsSource).unwrap().0;
    s.scroll.snap((row.y - 180.0).max(0.0));
    for (name, policy) in [("never", crate::lyric_filter::Policy::Never), ("automatic", crate::lyric_filter::Policy::Automatic), ("manual", crate::lyric_filter::Policy::Manual)] {
        super::super::act(&mut app, Act::LyricsSource);
        assert_eq!(app.cfg.lyrics_sources.get("mpv.exe"), Some(&policy));
        let saved: Config = config::load_json("config.json");
        assert_eq!(saved.lyrics_sources, app.cfg.lyrics_sources);
        for scale in [1.0, 1.5, 2.0] { snapshot(&mut app, &format!("lyric-searches-{name}"), scale); }
    }
    app.media.as_mut().unwrap().info.app_id = "chrome.exe".into();
    app.cfg.lyrics_sources.insert("chrome.exe".into(), crate::lyric_filter::Policy::Automatic);
    for expected in [crate::lyric_filter::Policy::Never, crate::lyric_filter::Policy::Manual] {
        super::super::act(&mut app, Act::LyricsSource);
        assert_eq!(crate::lyric_filter::policy(&app.cfg, &app.media.as_ref().unwrap().info), expected);
        assert_eq!(app.cfg.lyrics_sources.get("chrome.exe"), Some(&expected));
    }
    for scale in [1.0, 1.5, 2.0] { snapshot(&mut app, "browser-manual-gate", scale); }
    unsafe {
        DestroyWindow(app.settings.take().unwrap().hwnd).unwrap();
        DestroyWindow(app.notch.hwnd).unwrap();
    }
}

#[test]
#[ignore = "native graphics; requires isolated APPDATA ending in sound-test"]
fn about_debug_previews_render_and_dispatch() {
    let mut app = fixture();
    std::fs::create_dir_all(config::data_dir()).unwrap();
    app.settings.as_mut().unwrap().page = ABOUT_PAGE;
    app.settings.as_mut().unwrap().side_sel.snap(ABOUT_PAGE as f32);
    snapshot(&mut app, "about-debug-initial", 1.0);
    let s = app.settings.as_mut().unwrap();
    let row = s.hits.iter().find(|(_, a)| *a == Act::DebugUpdates).unwrap().0;
    s.scroll.snap((row.y - 145.0).max(0.0));
    for scale in [1.0, 1.5, 2.0] { snapshot(&mut app, "about-debug", scale); }
    let original = app.cfg.clone();
    super::super::act(&mut app, Act::DebugUpdates);
    assert!(app.notch.update_preview.is_some());
    assert_eq!(app.notch.page, crate::notch::Page::Updates);
    assert!(matches!(app.update, Status::Idle));
    super::super::act(&mut app, Act::DebugHud);
    assert!(app.notch.update_preview.is_none());
    assert!(matches!(app.notch.hud, Some((crate::notch::Hud::Volume { level: 0.5, muted: false }, _))));
    super::super::act(&mut app, Act::DebugNotice);
    assert!(app.notch.notice.as_ref().unwrap().0.contains("Test notification"));
    assert_eq!(app.cfg, original);
    unsafe {
        DestroyWindow(app.settings.take().unwrap().hwnd).unwrap();
        DestroyWindow(app.notch.hwnd).unwrap();
    }
}
