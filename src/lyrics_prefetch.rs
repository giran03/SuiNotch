//! Lyrics fetched ahead: while a song plays, the one likely to follow (learned in `track_history`, or
//! the next track of the album) is looked up into the cache, so its lyrics show the moment it starts.
//! It runs in the prefetch lane (see `lyrics_net`), giving way to the song playing, at most one at a
//! time, a limited number an hour, and not on a metered connection. It only ever writes the cache.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::events::{MediaInfo, MediaKind};
use crate::lyrics::{self, Options, Request};
use crate::lyrics_net::{self, Lane};
use crate::{localized, lyrics_cache, lyrics_local, lyrics_log};

/// Wait this long into a song (or this share of it, if sooner) before looking ahead, so skipping
/// through a playlist costs nothing.
const MIN_PLAYED_MS: u64 = 30_000;
const MIN_PLAYED_SHARE: f64 = 0.4;
const PER_HOUR: usize = 30;
const HOUR: Duration = Duration::from_secs(3600);

/// Lookups ahead made in the last hour.
static SPENT: Mutex<VecDeque<Instant>> = Mutex::new(VecDeque::new());

#[derive(Clone, Debug)]
pub enum Target {
    /// A song known to follow, as the player named it last time.
    Known(MediaInfo),
    /// Whatever follows this track on its album, asked of the store.
    AlbumNext(MediaInfo),
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub target: Target,
    pub wait: Duration,
    pub opts: Options,
    /// Lyrics marked wrong, per song (as in `lyrics::Shifts`), to skip for the song ahead once known.
    pub blocked: std::collections::HashMap<String, Vec<String>>,
}

/// How long until a song at `position_ms` has played enough to look ahead.
pub fn wait_for(position_ms: u64, duration_ms: u64) -> Duration {
    let enough = if duration_ms > 0 { MIN_PLAYED_MS.min((duration_ms as f64 * MIN_PLAYED_SHARE) as u64) } else { MIN_PLAYED_MS };
    Duration::from_millis(enough.saturating_sub(position_ms))
}

/// Takes one lookup from the hourly allowance; false when it is used up.
fn spend(spent: &mut VecDeque<Instant>, now: Instant) -> bool {
    while spent.front().is_some_and(|t| now.duration_since(*t) >= HOUR) {
        spent.pop_front();
    }
    if spent.len() >= PER_HOUR {
        return false;
    }
    spent.push_back(now);
    true
}

/// A connection that costs by the byte, has a cap, or is roaming.
fn metered() -> bool {
    use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
    let Ok(cost) = NetworkInformation::GetInternetConnectionProfile().and_then(|p| p.GetConnectionCost()) else { return false };
    matches!(cost.NetworkCostType(), Ok(NetworkCostType::Fixed | NetworkCostType::Variable))
        || cost.Roaming().unwrap_or(false)
        || cost.OverDataLimit().unwrap_or(false)
        || cost.ApproachingDataLimit().unwrap_or(false)
}

/// The song to look up, once known; None when there is none (or it is already known).
fn resolve_target(target: Target) -> Option<MediaInfo> {
    match target {
        Target::Known(info) => Some(info),
        Target::AlbumNext(current) => {
            let t = localized::album_track(&current)?;
            Some(MediaInfo { title: t.title, artist: t.artist, album: current.album.clone(), album_artist: current.album_artist.clone(), duration_ms: t.duration_ms, app_id: current.app_id.clone(), kind: MediaKind::Music, playing: true, ..Default::default() })
        }
    }
}

fn known(info: &MediaInfo, fallback: bool) -> bool {
    let load = |key: u64| lyrics_cache::load(key, info.duration_ms, fallback).is_some();
    load(info.track_key()) || load(lyrics_cache::alias_key(&info.title, &info.artist)) || lyrics_local::find(info).is_some()
}

fn label(target: &Target) -> String {
    match target {
        Target::Known(i) => format!("{} - {}", i.artist, i.title),
        Target::AlbumNext(i) => format!("after {} - {}", i.artist, i.title),
    }
}

/// Starts looking ahead, replacing any lookup ahead still waiting or running.
pub fn start(plan: Plan) {
    lyrics_net::cancel_prefetch();
    let generation = lyrics_net::next_prefetch();
    std::thread::spawn(move || {
        let _lane = lyrics_net::enter(Lane::Prefetch(generation));
        if !lyrics_net::sleep(plan.wait) {
            return;
        }
        if metered() {
            lyrics_log::skipped(label(&plan.target), "Not on a metered connection");
            return;
        }
        // Known songs already cached cost nothing; an album lookup costs from the start.
        let fallback = plan.opts.fallback;
        if let Target::Known(info) = &plan.target {
            if known(info, fallback) {
                return;
            }
        }
        // Songs named by the store are kept under their alias, which the player's spelling also finds.
        let alias = matches!(plan.target, Target::AlbumNext(_));
        if !spend(&mut SPENT.lock().unwrap_or_else(|e| e.into_inner()), Instant::now()) {
            lyrics_log::skipped(label(&plan.target), "Hourly limit for lookups ahead reached");
            return;
        }
        let Some(info) = resolve_target(plan.target) else { return };
        if !lyrics_net::live() || known(&info, fallback) {
            return;
        }
        let skip = plan.blocked.get(&info.track_key().to_string()).cloned().unwrap_or_default();
        // One round only: a song ahead is not worth retrying for.
        let req = Request { info, attempt: 0, opts: Options { retries: 0, ..plan.opts }, automatic: true, verify: false, skip, alias };
        let started = Instant::now();
        if let Some(outcome) = lyrics::resolve(&req, Duration::ZERO) {
            lyrics_log::note(&req.info, "Ahead", &outcome, started);
        }
    });
}

pub fn cancel() {
    lyrics_net::cancel_prefetch();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_ahead_waits_until_the_song_has_played_a_while() {
        assert_eq!(wait_for(0, 200_000), Duration::from_secs(30));
        assert_eq!(wait_for(10_000, 200_000), Duration::from_secs(20));
        assert_eq!(wait_for(90_000, 200_000), Duration::ZERO);
        // A short song waits 40 % of it instead.
        assert_eq!(wait_for(0, 50_000), Duration::from_secs(20));
        assert_eq!(wait_for(0, 0), Duration::from_secs(30));
    }

    #[test]
    fn the_hourly_allowance_runs_out_and_comes_back() {
        let mut spent = VecDeque::new();
        let t = Instant::now();
        for _ in 0..PER_HOUR {
            assert!(spend(&mut spent, t));
        }
        assert!(!spend(&mut spent, t + Duration::from_secs(10)));
        assert!(spend(&mut spent, t + HOUR), "an hour later the oldest have expired");
    }
}
