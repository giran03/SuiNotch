//! Now-playing information from any app that uses Windows media controls
//! (Spotify, Apple Music, browsers, ...), plus transport commands.

use std::sync::OnceLock;
use std::sync::Mutex;

use windows::core::Result;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

use crate::events::{post, Event, MediaInfo};

static MANAGER: OnceLock<Manager> = OnceLock::new();
static SUBSCRIBED: Mutex<Option<(Session, [i64; 3])>> = Mutex::new(None);
static LAST_ART_KEY: Mutex<u64> = Mutex::new(0);
const ART_MAX_PX: u32 = 256;
/// Timeline updates within this much of where playback should be are not worth a redraw.
const POSITION_SLACK_MS: u64 = 1500;

/// Title, artist, album and player of the current track, read only when the track changes.
#[derive(Clone)]
struct Track {
    title: String,
    artist: String,
    album: String,
    app_id: String,
}

static TRACK: Mutex<Option<Track>> = Mutex::new(None);
static LAST_POSTED: Mutex<Option<(MediaInfo, std::time::Instant)>> = Mutex::new(None);

#[derive(Clone, Copy, Debug)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    Seek(u64),
}

pub fn start() {
    std::thread::spawn(|| {
        if let Err(e) = run() {
            crate::log(&format!("media controls unavailable: {e}"));
        }
    });
}

fn run() -> Result<()> {
    let manager = Manager::RequestAsync()?.get()?;
    manager.CurrentSessionChanged(&TypedEventHandler::new(|_, _| {
        resubscribe();
        Ok(())
    }))?;
    manager.SessionsChanged(&TypedEventHandler::new(|_, _| {
        resubscribe();
        Ok(())
    }))?;
    let _ = MANAGER.set(manager);
    resubscribe();
    Ok(())
}

fn resubscribe() {
    let Some(manager) = MANAGER.get() else { return };
    let current = manager.GetCurrentSession().ok();
    let mut sub = SUBSCRIBED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((old, tokens)) = sub.take() {
        let _ = old.RemoveMediaPropertiesChanged(tokens[0]);
        let _ = old.RemovePlaybackInfoChanged(tokens[1]);
        let _ = old.RemoveTimelinePropertiesChanged(tokens[2]);
    }
    if let Some(session) = current {
        let t0 = session.MediaPropertiesChanged(&TypedEventHandler::new(|_, _| {
            refresh(true);
            Ok(())
        }));
        let t1 = session.PlaybackInfoChanged(&TypedEventHandler::new(|_, _| {
            refresh(false);
            Ok(())
        }));
        let t2 = session.TimelinePropertiesChanged(&TypedEventHandler::new(|_, _| {
            refresh(false);
            Ok(())
        }));
        if let (Ok(a), Ok(b), Ok(c)) = (t0, t1, t2) {
            *sub = Some((session, [a, b, c]));
        }
    }
    drop(sub);
    refresh(true);
}

fn filetime_now() -> i64 {
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

fn track(session: &Session, reload: bool) -> Result<Track> {
    let mut cached = TRACK.lock().unwrap_or_else(|e| e.into_inner());
    if let (false, Some(t)) = (reload, cached.as_ref()) {
        return Ok(t.clone());
    }
    let props = session.TryGetMediaPropertiesAsync()?.get()?;
    let t = Track {
        title: props.Title()?.to_string(),
        artist: props.Artist()?.to_string(),
        album: props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default(),
        app_id: session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default(),
    };
    *cached = Some(t.clone());
    Ok(t)
}

fn snapshot(session: &Session, reload: bool) -> Result<MediaInfo> {
    let t = track(session, reload)?;
    let playback = session.GetPlaybackInfo()?;
    let timeline = session.GetTimelineProperties()?;
    let playing = playback.PlaybackStatus()? == Status::Playing;
    let duration_ms = (timeline.EndTime()?.Duration / 10_000).max(0) as u64;
    let mut position = timeline.Position()?.Duration;
    if playing {
        // Position is reported as of LastUpdatedTime; move it forward to now.
        let updated = timeline.LastUpdatedTime()?.UniversalTime;
        if updated > 0 {
            position += (filetime_now() - updated).max(0);
        }
    }
    let position_ms = ((position / 10_000).max(0) as u64).min(duration_ms.max(1));
    Ok(MediaInfo {
        title: t.title,
        artist: t.artist,
        album: t.album,
        app_id: t.app_id,
        playing,
        position_ms,
        duration_ms,
    })
}

fn read_thumbnail(session: &Session) -> Result<Vec<u8>> {
    let props = session.TryGetMediaPropertiesAsync()?.get()?;
    let stream = props.Thumbnail()?.OpenReadAsync()?.get()?;
    let size = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    reader.LoadAsync(size)?.get()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}

fn refresh(with_art: bool) {
    let session = MANAGER.get().and_then(|m| m.GetCurrentSession().ok());
    let Some(session) = session else {
        post(Event::Media(None));
        return;
    };
    match snapshot(&session, with_art) {
        Ok(info) if !info.title.is_empty() => {
            let key = info.track_key();
            if worth_posting(&info) {
                post(Event::Media(Some(info)));
            }
            let mut last = LAST_ART_KEY.lock().unwrap_or_else(|e| e.into_inner());
            if with_art || *last != key {
                *last = key;
                drop(last);
                // Some players publish the artwork a moment after the title.
                let bytes = read_thumbnail(&session).ok().filter(|b| !b.is_empty());
                // Decode here, not on the UI thread, so a track change never stalls an animation.
                let art = bytes.and_then(|b| crate::gfx::decode_art(&b, ART_MAX_PX));
                post(Event::Artwork { key, art });
            }
        }
        _ => {
            *LAST_POSTED.lock().unwrap_or_else(|e| e.into_inner()) = None;
            post(Event::Media(None));
        }
    }
}

/// Skips updates that only confirm where playback already is, so steady playback costs no redraws.
fn worth_posting(info: &MediaInfo) -> bool {
    let mut last = LAST_POSTED.lock().unwrap_or_else(|e| e.into_inner());
    let unchanged = last.as_ref().is_some_and(|(prev, at)| {
        let expected = prev.position_ms + if prev.playing { at.elapsed().as_millis() as u64 } else { 0 };
        let same_track = MediaInfo { position_ms: 0, ..prev.clone() } == MediaInfo { position_ms: 0, ..info.clone() };
        same_track && info.position_ms.abs_diff(expected) < POSITION_SLACK_MS
    });
    if !unchanged {
        *last = Some((info.clone(), std::time::Instant::now()));
    }
    !unchanged
}

pub fn command(cmd: Command) {
    std::thread::spawn(move || {
        let Some(session) = MANAGER.get().and_then(|m| m.GetCurrentSession().ok()) else { return };
        let result = match cmd {
            Command::PlayPause => session.TryTogglePlayPauseAsync().and_then(|op| op.get()),
            Command::Next => session.TrySkipNextAsync().and_then(|op| op.get()),
            Command::Previous => session.TrySkipPreviousAsync().and_then(|op| op.get()),
            Command::Seek(ms) => session.TryChangePlaybackPositionAsync(ms as i64 * 10_000).and_then(|op| op.get()),
        };
        if let Err(e) = result {
            crate::log(&format!("media command {cmd:?} failed: {e}"));
        }
        refresh(false);
    });
}

/// Friendly player name from an AppUserModelID such as "Spotify.exe" or "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic".
pub fn source_name(app_id: &str) -> String {
    let lower = app_id.to_ascii_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("applemusic", "Apple Music"),
        ("zunemusic", "Media Player"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("vlc", "VLC"),
        ("foobar", "foobar2000"),
        ("tidal", "TIDAL"),
        ("deezer", "Deezer"),
        ("youtube", "YouTube Music"),
    ];
    for (needle, name) in known {
        if lower.contains(needle) {
            return name.to_string();
        }
    }
    let base = app_id.rsplit(['!', '\\', '/']).next().unwrap_or(app_id);
    base.trim_end_matches(".exe").to_string()
}

#[cfg(test)]
mod tests {
    use super::source_name;

    #[test]
    fn friendly_source_names() {
        assert_eq!(source_name("Spotify.exe"), "Spotify");
        assert_eq!(source_name("AppleInc.AppleMusicWin_nzyj5cx40ttqa!App"), "Apple Music");
        assert_eq!(source_name("C:\\Tools\\mpv.exe"), "mpv");
    }
}
