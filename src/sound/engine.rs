//! All file decoding and playback run on background threads; one audio channel owns playback.
use super::{synth, RATE};
use rodio::{buffer::SamplesBuffer, Decoder, OutputStream, Sink, Source};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
pub const EVENT_LABELS: [&str; 7] =
    ["Focus complete", "Break complete", "Timer complete", "Task reminder", "Calendar reminder", "Water reminder", "Low battery"];
pub const DEFAULTS: [&str; 7] = ["bell", "soft", "bell", "bell", "bell", "drop", "bell"];
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub seconds: f32,
}
pub const PRESETS: [Preset; 8] = [
    Preset { id: "drop", name: "Sui Drop", description: "A round water drop", seconds: 0.35 },
    Preset { id: "bell", name: "Bell", description: "Two rising notes", seconds: 1.3 },
    Preset { id: "soft", name: "Soft", description: "A gentle descending pair", seconds: 1.2 },
    Preset { id: "glass", name: "Glass", description: "A clear, delicate chime", seconds: 0.9 },
    Preset { id: "ripple", name: "Ripple", description: "A gentle double pulse", seconds: 1.1 },
    Preset { id: "spark", name: "Spark", description: "A short, bright ping", seconds: 0.45 },
    Preset { id: "beacon", name: "Beacon", description: "A noticeable melodic phrase", seconds: 2.0 },
    Preset { id: "ring", name: "Classic Ring", description: "A familiar ringing pattern", seconds: 2.4 },
];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub volume: u32,
    pub assignments: [String; 7],
    pub custom: Vec<CustomSound>,
    pub timer_repeat: bool,
    pub repeat_gap_s: u32,
    pub ring_limit_s: u32,
    pub gradual: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: 100,
            assignments: DEFAULTS.map(str::to_owned),
            custom: Vec::new(),
            timer_repeat: false,
            repeat_gap_s: 3,
            ring_limit_s: 60,
            gradual: false,
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CustomSound {
    pub id: String,
    pub name: String,
    pub duration_ms: u32,
    pub start_ms: u32,
    pub length_ms: u32,
    pub normalize: bool,
}
impl CustomSound {
    pub fn sanitize(&mut self) {
        self.name = self.name.chars().filter(|c| !c.is_control()).take(60).collect();
        if self.name.trim().is_empty() {
            self.name = "Custom sound".into();
        }
        self.duration_ms = self.duration_ms.clamp(100, 300_000);
        self.start_ms = self.start_ms.min(self.duration_ms - 100);
        self.length_ms = self.length_ms.clamp(100, 30_000).min(self.duration_ms - self.start_ms);
    }
}
impl Settings {
    pub fn sanitize(&mut self) {
        self.volume = self.volume.min(100);
        self.repeat_gap_s = self.repeat_gap_s.clamp(1, 30);
        self.ring_limit_s = self.ring_limit_s.clamp(5, 300);
        let mut seen = std::collections::HashSet::new();
        self.custom.retain(|s| valid_id(&s.id) && seen.insert(s.id.clone()));
        for s in &mut self.custom {
            s.sanitize();
        }
        for (i, id) in self.assignments.iter_mut().enumerate() {
            if id != "silent" && !PRESETS.iter().any(|p| p.id == id) && !self.custom.iter().any(|s| s.id == *id) {
                *id = DEFAULTS[i].into();
            }
        }
    }
    pub fn choices(&self) -> Vec<(String, String)> {
        std::iter::once(("silent".into(), "Silent".into()))
            .chain(PRESETS.iter().map(|p| (p.id.into(), p.name.into())))
            .chain(self.custom.iter().map(|s| (s.id.clone(), s.name.clone())))
            .collect()
    }
    pub fn name(&self, id: &str) -> String {
        self.choices().into_iter().find(|(k, _)| k == id).map_or_else(|| "Unavailable sound".into(), |(_, n)| n)
    }
    pub fn reset_assignments(&mut self) {
        self.assignments = DEFAULTS.map(str::to_owned);
    }
    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        let Some(sound) = self.custom.get(index) else { return Err("Sound no longer exists".into()) };
        let path = asset_path(&sound.id).ok_or("Invalid sound ID")?;
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not remove the sound file. Try again.".into()),
        }
        let sound = self.custom.remove(index);
        for (i, id) in self.assignments.iter_mut().enumerate() {
            if *id == sound.id {
                *id = DEFAULTS[i].into();
            }
        }
        Ok(())
    }
}
fn valid_id(id: &str) -> bool {
    id.starts_with("custom-") && (8..=80).contains(&id.len()) && id[7..].bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}
fn asset_path(id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| crate::config::data_dir().join("sounds").join(format!("{id}.audio")))
}
pub fn available(id: &str) -> bool {
    asset_path(id).is_some_and(|p| p.is_file())
}

#[derive(Clone)]
struct Clip {
    channels: u16,
    rate: u32,
    samples: Vec<f32>,
}
#[cfg(test)]
fn decode(bytes: Vec<u8>, token: Option<u64>) -> Result<Clip, String> {
    decode_window(bytes, token, None, true).map(|(clip, _)| clip)
}
// Probe long sources without retaining their decoded samples; playback retains only the selected clip.
fn decode_window(bytes: Vec<u8>, token: Option<u64>, window: Option<(u32, u32)>, retain: bool) -> Result<(Clip, u32), String> {
    std::panic::catch_unwind(|| decode_inner(bytes, token, window, retain))
        .unwrap_or_else(|_| Err("Could not decode this audio file. Try another file.".into()))
}
fn decode_inner(bytes: Vec<u8>, token: Option<u64>, window: Option<(u32, u32)>, retain: bool) -> Result<(Clip, u32), String> {
    let source = Decoder::new(Cursor::new(bytes)).map_err(|_| "This file is not supported audio. Choose WAV, MP3 or OGG Vorbis.")?;
    let (channels, rate) = (source.channels(), source.sample_rate());
    if !(1..=2).contains(&channels) || !(8_000..=96_000).contains(&rate) {
        return Err("Use mono or stereo audio between 8 and 96 kHz.".into());
    }
    let max = 300 * rate as usize * channels as usize;
    let (begin, end) = window.map_or((0, max), |(start, length)| {
        let begin = (start as u64 * rate as u64 / 1000) as usize * channels as usize;
        (begin, begin + (length.min(30_000) as u64 * rate as u64 / 1000) as usize * channels as usize)
    });
    let mut samples = Vec::new();
    let mut count = 0;
    for (i, s) in source.convert_samples::<f32>().enumerate() {
        if window.is_some() && i >= end {
            break;
        }
        if i >= max {
            return Err("Source audio is too long. Choose a file under five minutes.".into());
        }
        if i % 4096 == 0 && token.is_some_and(|t| GENERATION.load(Ordering::SeqCst) != t) {
            return Err("Cancelled".into());
        }
        count += 1;
        if retain && i >= begin {
            samples.push(if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 });
        }
    }
    samples.truncate(samples.len() / channels as usize * channels as usize);
    if count == 0 || (retain && samples.is_empty()) {
        return Err("The audio file contains no playable samples.".into());
    }
    let duration = (count as u64 * 1000 / channels as u64 / rate as u64) as u32;
    Ok((Clip { channels, rate, samples }, duration))
}
fn read_audio(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|_| "Could not open the sound file.")?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|_| "Could not read the sound file.")?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err("Choose an audio file smaller than 20 MB.".into());
    }
    Ok(bytes)
}
fn crop(mut clip: Clip, sound: &CustomSound) -> Clip {
    let frame = clip.channels as usize;
    let begin = (sound.start_ms as u64 * clip.rate as u64 / 1000) as usize * frame;
    let len = (sound.length_ms.min(30_000) as u64 * clip.rate as u64 / 1000) as usize * frame;
    clip.samples = clip.samples.into_iter().skip(begin).take(len).collect();
    if sound.normalize {
        let peak = clip.samples.iter().fold(0.0f32, |p, s| p.max(s.abs()));
        if peak > 0.001 {
            let gain = (0.7 / peak).min(4.0);
            for s in &mut clip.samples {
                *s = (*s * gain).clamp(-0.9, 0.9);
            }
        }
    }
    let fade_frames = (clip.rate as usize / 200).min(clip.samples.len() / frame / 2);
    let count = clip.samples.len();
    for i in 0..fade_frames * frame {
        let gain = (i / frame) as f32 / fade_frames.max(1) as f32;
        clip.samples[i] *= gain;
        clip.samples[count - 1 - i] *= gain;
    }
    clip
}
fn preset(id: &str) -> Clip {
    let bytes = match id {
        "drop" => synth::drop_sound(),
        "soft" => synth::wav(&[(988.0, 0.0), (740.0, 0.18)], 1.2),
        "glass" => synth::wav(&[(1568.0, 0.0)], 0.9),
        "ripple" => synth::wav(&[(523.0, 0.0), (659.0, 0.25)], 1.1),
        "spark" => synth::wav(&[(2093.0, 0.0)], 0.45),
        "beacon" => synth::wav(&[(659.0, 0.0), (784.0, 0.3), (988.0, 0.6)], 2.0),
        "ring" => synth::wav(&[(880.0, 0.0), (1175.0, 0.12), (880.0, 0.6), (1175.0, 0.72), (880.0, 1.2), (1175.0, 1.32)], 2.4),
        _ => synth::wav(&[(784.0, 0.0), (1175.0, 0.16)], 1.3),
    };
    let samples = bytes[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
    Clip { channels: 1, rate: RATE, samples }
}

pub fn import(path: PathBuf, replacement: Option<String>) {
    std::thread::spawn(move || {
        let result = import_file(&path);
        crate::events::post(crate::events::Event::SoundImported { result, replacement });
    });
}
fn import_file(path: &Path) -> Result<CustomSound, String> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["wav", "mp3", "ogg"].contains(&ext.as_str()) {
        return Err("Choose a WAV, MP3 or OGG file.".into());
    }
    let bytes = read_audio(path)?;
    let (_, duration_ms) = decode_window(bytes.clone(), None, None, false)?;
    if duration_ms < 100 {
        return Err("Choose audio at least 0.1 seconds long.".into());
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = format!("custom-{:x}-{:x}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos(), NEXT.fetch_add(1, Ordering::Relaxed));
    let dest = asset_path(&id).ok_or("Invalid sound ID")?;
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|_| "Could not create the local sound library.")?;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&dest).map_err(|_| "Could not save the imported sound.")?;
    if file.write_all(&bytes).and_then(|_| file.sync_all()).is_err() {
        drop(file);
        let _ = std::fs::remove_file(dest);
        return Err("Could not save the imported sound.".into());
    }
    let mut sound = CustomSound {
        id,
        name: path.file_stem().unwrap_or_default().to_string_lossy().into(),
        duration_ms,
        start_ms: 0,
        length_ms: duration_ms.min(30_000),
        normalize: false,
    };
    sound.sanitize();
    Ok(sound)
}

struct Request {
    token: u64,
    id: String,
    custom: Option<CustomSound>,
    fallback: &'static str,
    volume: f32,
    repeat: Option<(u32, u32, bool)>,
    respect_quiet: bool,
}
enum Command {
    Play(Request),
    Stop,
}
static PLAYER: OnceLock<mpsc::Sender<Command>> = OnceLock::new();
static GENERATION: AtomicU64 = AtomicU64::new(0);
// 0 idle, 1 preview, 2 reminder, 3 repeating timer.
static PLAYING: AtomicU8 = AtomicU8::new(0);
fn send(command: Command) {
    let sender = PLAYER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || player(rx));
        tx
    });
    let _ = sender.send(command);
}
pub fn stop() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    PLAYING.store(0, Ordering::SeqCst);
    if let Some(tx) = PLAYER.get() {
        let _ = tx.send(Command::Stop);
    }
}
pub fn stop_preview() {
    if PLAYING.load(Ordering::SeqCst) == 1 {
        stop();
    }
}
pub fn previewing() -> bool {
    PLAYING.load(Ordering::SeqCst) == 1
}
pub fn ringing() -> bool {
    PLAYING.load(Ordering::SeqCst) == 3
}
pub fn alert_playing() -> bool {
    PLAYING.load(Ordering::SeqCst) >= 2
}
pub fn preview(settings: &Settings, id: &str) {
    if PLAYING.load(Ordering::SeqCst) >= 2 {
        return;
    }
    play(settings, id, "bell", false, true, false);
}
pub fn alert(cfg: &crate::config::Config, event: usize) {
    stop();
    if !cfg.sounds || (cfg.respect_quiet && crate::quiet::active()) {
        return;
    }
    play(&cfg.sound_library, &cfg.sound_library.assignments[event], DEFAULTS[event], event == 2 && cfg.sound_library.timer_repeat, false, cfg.respect_quiet);
}
fn play(s: &Settings, id: &str, fallback: &'static str, repeat: bool, preview: bool, respect_quiet: bool) {
    if id == "silent" || s.volume == 0 {
        return;
    }
    let token = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    PLAYING.store(
        if preview {
            1
        } else if repeat {
            3
        } else {
            2
        },
        Ordering::SeqCst,
    );
    send(Command::Play(Request {
        token,
        id: id.into(),
        custom: s.custom.iter().find(|c| c.id == id).cloned(),
        fallback,
        volume: s.volume.min(100) as f32 / 100.0,
        repeat: repeat.then_some((s.repeat_gap_s.clamp(1, 30), s.ring_limit_s.clamp(5, 300), s.gradual)),
        respect_quiet,
    }));
}
fn report(message: &str) {
    crate::events::post(crate::events::Event::SoundMessage(message.into()));
}
#[derive(Debug, PartialEq)]
enum RingAction {
    Wait,
    Play,
    Stop,
}
struct RingSchedule {
    gap: Duration,
    limit: Duration,
    next: Duration,
}
impl RingSchedule {
    fn new(gap: u32, limit: u32) -> Self {
        Self { gap: Duration::from_secs(gap as u64), limit: Duration::from_secs(limit as u64), next: Duration::ZERO }
    }
    fn poll(&mut self, elapsed: Duration, playing: bool) -> RingAction {
        if elapsed >= self.limit {
            return RingAction::Stop;
        }
        if playing {
            self.next = elapsed + self.gap;
            return RingAction::Wait;
        }
        if elapsed >= self.next {
            self.next = elapsed + self.gap;
            RingAction::Play
        } else {
            RingAction::Wait
        }
    }
}
struct Active {
    _stream: OutputStream,
    sink: Sink,
    clip: Clip,
    request: Request,
    started: Instant,
    ring: Option<RingSchedule>,
    quiet_check: Instant,
}
fn player(rx: mpsc::Receiver<Command>) {
    let mut active: Option<Active> = None;
    loop {
        let command = if active.is_some() {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(c) => Some(c),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(_) => break,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => break,
            }
        };
        match command {
            Some(Command::Stop) => {
                active = None;
            }
            Some(Command::Play(request)) => {
                active = None;
                if GENERATION.load(Ordering::SeqCst) != request.token {
                    continue;
                }
                let clip = match &request.custom {
                    Some(sound) => asset_path(&sound.id)
                        .ok_or_else(|| "Invalid sound ID".into())
                        .and_then(|p| read_audio(&p))
                        .and_then(|b| decode_window(b, Some(request.token), Some((sound.start_ms, sound.length_ms)), true))
                        .map(|(c, _)| {
                            let mut selected = sound.clone();
                            selected.start_ms = 0;
                            crop(c, &selected)
                        }),
                    None if PRESETS.iter().any(|p| p.id == request.id) => Ok(preset(&request.id)),
                    None => Err("Sound unavailable".into()),
                };
                if GENERATION.load(Ordering::SeqCst) != request.token {
                    continue;
                }
                let clip = match clip {
                    Ok(c) if !c.samples.is_empty() => c,
                    _ => {
                        report("Custom sound unavailable; playing the default chime.");
                        preset(request.fallback)
                    }
                };
                let output = OutputStream::try_default()
                    .map_err(|e| e.to_string())
                    .and_then(|(stream, handle)| Sink::try_new(&handle).map(|sink| (stream, sink)).map_err(|e| e.to_string()));
                match output {
                    Ok((stream, sink)) => {
                        if GENERATION.load(Ordering::SeqCst) != request.token {
                            continue;
                        }
                        sink.set_volume(if request.repeat.is_some_and(|r| r.2) { request.volume * 0.15 } else { request.volume });
                        sink.append(SamplesBuffer::new(clip.channels, clip.rate, clip.samples.clone()));
                        let started = Instant::now();
                        let ring = request.repeat.map(|(gap, limit, _)| RingSchedule::new(gap, limit));
                        active = Some(Active { _stream: stream, sink, clip, request, started, ring, quiet_check: started });
                    }
                    Err(e) => {
                        crate::log(&format!("sound output unavailable: {e}"));
                        if GENERATION.load(Ordering::SeqCst) == request.token {
                            PLAYING.store(0, Ordering::SeqCst);
                            report("Sound output unavailable. Check your audio device.");
                        }
                    }
                }
            }
            None => {}
        }
        if let Some(a) = &mut active {
            let elapsed = a.started.elapsed();
            let stale = GENERATION.load(Ordering::SeqCst) != a.request.token;
            let ring_action = a.ring.as_mut().map(|r| r.poll(elapsed, !a.sink.empty()));
            let done = ring_action.as_ref().map_or_else(|| a.sink.empty(), |action| *action == RingAction::Stop);
            let quiet = a.request.respect_quiet && a.quiet_check.elapsed() >= Duration::from_millis(250) && {
                a.quiet_check = Instant::now();
                crate::quiet::active()
            };
            if stale || done || quiet {
                if !stale {
                    PLAYING.store(0, Ordering::SeqCst);
                }
                active = None;
                crate::events::wake(crate::events::WM_APP_EVENT);
                continue;
            }
            if let Some((_, _, gradual)) = a.request.repeat {
                if gradual {
                    a.sink.set_volume(a.request.volume * (0.15 + elapsed.as_secs_f32() / 10.0).min(1.0));
                }
                if ring_action == Some(RingAction::Play) {
                    a.sink.append(SamplesBuffer::new(a.clip.channels, a.clip.rate, a.clip.samples.clone()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a Windows output device and isolated APPDATA ending in sound-test; plays only silence"]
    fn native_playback_stops_at_limit_and_previews_cannot_interrupt_timer() {
        let dir = crate::config::data_dir();
        assert!(dir.parent().unwrap().ends_with("sound-test"));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("silent-playback-test.wav");
        std::fs::write(&source, synth::wav(&[], 0.2)).unwrap();
        let custom = import_file(&source).unwrap();
        std::fs::remove_file(source).unwrap();
        let mut cfg = crate::config::Config { respect_quiet: false, ..Default::default() };
        cfg.sound_library.custom.push(custom.clone());
        cfg.sound_library.assignments[2] = custom.id.clone();
        cfg.sound_library.timer_repeat = true;
        cfg.sound_library.ring_limit_s = 5;
        cfg.sound_library.repeat_gap_s = 1;
        alert(&cfg, 2);
        std::thread::sleep(Duration::from_millis(500));
        assert!(ringing(), "Windows audio output could not start");
        preview(&cfg.sound_library, &custom.id);
        stop_preview();
        assert!(ringing(), "preview controls must not stop a timer");
        let deadline = Instant::now() + Duration::from_secs(7);
        while ringing() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!ringing(), "ringing exceeded its configured time limit");
        preview(&cfg.sound_library, &custom.id);
        assert!(previewing());
        stop_preview();
        assert!(!previewing());
        alert(&cfg, 2);
        stop();
        assert!(!ringing());
        cfg.sound_library.remove(0).unwrap();
    }
    #[test]
    fn repetition_waits_for_clip_and_gap_and_always_stops_at_limit() {
        let mut ring = RingSchedule::new(3, 60);
        assert_eq!(ring.poll(Duration::from_secs(1), true), RingAction::Wait);
        assert_eq!(ring.poll(Duration::from_secs(2), true), RingAction::Wait);
        assert_eq!(ring.poll(Duration::from_secs(4), false), RingAction::Wait);
        assert_eq!(ring.poll(Duration::from_secs(5), false), RingAction::Play);
        assert_eq!(ring.poll(Duration::from_secs(6), true), RingAction::Wait);
        assert_eq!(ring.poll(Duration::from_secs(60), true), RingAction::Stop);
        assert_eq!(ring.poll(Duration::from_secs(90), false), RingAction::Stop);
    }
    #[test]
    fn legacy_settings_keep_chimes_and_repetition_off() {
        let c: crate::config::Config = serde_json::from_str("{\"sounds\":false}").unwrap();
        assert!(!c.sounds);
        assert_eq!(c.sound_library.assignments, DEFAULTS.map(str::to_owned));
        assert!(!c.sound_library.timer_repeat);
    }
    #[test]
    fn unsafe_ids_and_missing_assignments_are_repaired() {
        let mut s = Settings::default();
        s.custom.push(CustomSound {
            id: "custom-../../bad".into(),
            name: "Bad".into(),
            duration_ms: 0,
            start_ms: u32::MAX,
            length_ms: u32::MAX,
            normalize: false,
        });
        s.assignments[0] = "custom-../../bad".into();
        s.volume = u32::MAX;
        s.sanitize();
        assert!(s.custom.is_empty());
        assert_eq!(s.assignments[0], "bell");
        assert_eq!(s.volume, 100);
        assert!(asset_path("../outside").is_none());
    }
    #[test]
    fn every_preset_has_finite_distinct_bounded_audio() {
        let mut hashes = std::collections::HashSet::new();
        for p in PRESETS {
            let c = preset(p.id);
            assert!(!c.samples.is_empty());
            assert!(c.samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
            assert!((c.samples.len() as f32 / c.rate as f32 - p.seconds).abs() < 0.002);
            hashes.insert(c.samples.iter().take(6000).map(|s| s.to_bits()).collect::<Vec<_>>());
        }
        assert_eq!(hashes.len(), PRESETS.len());
    }
    #[test]
    fn invalid_audio_is_rejected_and_wav_decodes() {
        assert!(decode(b"not audio".to_vec(), None).is_err());
        let c = decode(synth::wav(&[(440.0, 0.0)], 0.1), None).unwrap();
        assert_eq!(c.channels, 1);
        assert_eq!(c.samples.len(), 4410);
    }
    #[test]
    fn all_three_import_formats_decode_real_audio() {
        // Original 440 Hz sine fixtures; encoded with ffmpeg, no third-party recordings.
        for bytes in
            [include_bytes!("fixtures/tone.wav").as_slice(), include_bytes!("fixtures/tone.mp3").as_slice(), include_bytes!("fixtures/tone.ogg").as_slice()]
        {
            let clip = decode(bytes.to_vec(), None).unwrap();
            assert_eq!(clip.channels, 1);
            assert_eq!(clip.rate, 16000);
            assert!((3500..=6000).contains(&clip.samples.len()));
            assert!(clip.samples.iter().any(|s| s.abs() > 0.01));
        }
    }
    #[test]
    #[ignore = "requires isolated APPDATA ending in sound-test"]
    fn imports_survive_source_moves_and_delete_resets_assignments() {
        let dir = crate::config::data_dir();
        assert!(dir.parent().unwrap().ends_with("sound-test"));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("temporary-source.wav");
        std::fs::write(&original, include_bytes!("fixtures/tone.wav")).unwrap();
        let imported = import_file(&original).unwrap();
        std::fs::remove_file(&original).unwrap();
        assert!(available(&imported.id));
        assert!(!decode(read_audio(&asset_path(&imported.id).unwrap()).unwrap(), None).unwrap().samples.is_empty());
        let mut settings = Settings::default();
        settings.assignments[0] = imported.id.clone();
        settings.assignments[5] = imported.id.clone();
        settings.custom.push(imported.clone());
        let mut restored: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        restored.sanitize();
        assert_eq!(restored.assignments[0], imported.id);
        restored.remove(0).unwrap();
        assert_eq!(restored.assignments[0], DEFAULTS[0]);
        assert_eq!(restored.assignments[5], DEFAULTS[5]);
        assert!(!available(&imported.id));
        std::fs::write(&original, b"invalid audio").unwrap();
        assert!(import_file(&original).is_err());
        std::fs::remove_file(&original).unwrap();
    }
    #[test]
    fn clip_bounds_and_normalization_are_safe() {
        let mut s = CustomSound { id: "custom-abcd".into(), name: "Test".into(), duration_ms: 2000, start_ms: 500, length_ms: 1000, normalize: true };
        let c = crop(Clip { channels: 2, rate: 8000, samples: vec![0.5; 32000] }, &s);
        assert_eq!(c.samples.len(), 16000);
        assert_eq!(c.samples[0], 0.0);
        assert!(c.samples.iter().all(|s| s.abs() <= 0.9));
        s.start_ms = u32::MAX;
        s.length_ms = u32::MAX;
        s.sanitize();
        assert!(s.start_ms + s.length_ms <= s.duration_ms);
    }
}
