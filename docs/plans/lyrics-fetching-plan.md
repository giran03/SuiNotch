# Plan: faster lyrics, next-track prefetch, and a better song gate

Status: implemented in 1.0.4, except §5.6 (Spotify queue, out of scope). Decisions taken: no "looks like a song"
hint for browsers; prefetch and learning on by default; no LRCLIB app registration yet (User-Agent identifies the
app). Catalog verdicts are remembered in memory for the session, not on disk. User docs:
[lyric-filtering.md](../lyric-filtering.md), [lyrics-prefetch.md](../lyrics-prefetch.md).

## 0. What the code does today

| Area | Today | Problem |
|---|---|---|
| Delay | `lyrics::fetch(.., delayed=true)` sleeps 2 s, **then** reads the disk cache | A cached song still waits 2 s before its lyrics appear |
| Name lookup | `names_for(.., resolve=fallback)` calls Apple's store (iTunes JP) **before** LRCLIB on every Latin-script title | One extra round trip on almost every song, even when LRCLIB has it under the original name |
| Retries | `1 + lyrics_retries` (default 6) rounds, 1 s apart, for any "nothing found" | A song LRCLIB/NetEase simply don't have costs ~6 × (iTunes + 2 LRCLIB + NetEase search + lyric calls) every time it plays. Network failures and "not found" are treated the same |
| Misses | Nothing is remembered | The same unknown song is searched again every play |
| HTTP | New `HttpClient` per lookup, no timeout | A stalled request blocks that worker indefinitely |
| User-Agent | `SuiNotch/1.0 (a living notch for Windows)` | LRCLIB counts clients without a homepage/contact as "generic" (~45 req/min per IP); identified ones get ~90 |
| Instrumentals | LRCLIB's `instrumental: true` is ignored | Shows "No lyrics found" and keeps retrying |
| Word timing | Only from NetEase's encrypted endpoint | LRCLIB now returns `lyricsfile` (YAML) with `hasWordSync`; word-level timings for some songs without NetEase |
| Next track | Nothing | Windows media controls (GSMTC) expose **no queue**, so the next song is unknown |
| Song gate | Per-player allow-list + `MediaKind::Video` + "Advertisement" title | Podcasts/audiobooks in Spotify, and talk/video in "Automatic" non-browser players, still get searched. Rich SMTC fields (`Genres`, `TrackNumber`, `AlbumArtist`, `AlbumTrackCount`, `Subtitle`) are never read |

## 1. The key constraint for prefetch

`GlobalSystemMediaTransportControlsSession` exposes the **current** track only. There is no "up next" in the
Windows API. So "detect the next track" has to come from somewhere else:

| Source of the next track | Accuracy | Cost / risk | Verdict |
|---|---|---|---|
| **A. Learned transitions**: remember "after A, B played" locally; when A starts, prefetch B | High for albums and playlists played in order, which is most listening; low for shuffle | Local only, small JSON file; one lookup per prediction | **Do it (core)** |
| **B. Album siblings**: when a track with an album plays, prefetch the rest of that album | Good for album listening | LRCLIB `/api/search` needs `q` or `track_name`, so it cannot list an album. Needs a track list from Apple's store (`entity=song`, album lookup), then one LRCLIB call per track. That is 10–20 requests per album | **Do it, but capped and lazy** (only the next track number, see §3.3) |
| **C. Spotify Web API** `GET /v1/me/player/queue` | Exact | OAuth PKCE, user must register their own Spotify developer app; development-mode apps are limited to a small allow-list of users, so it cannot ship as a default for everyone. Sends more data to Spotify | **Optional, last phase, opt-in, needs your call** |
| D. Watch position near the end | No identity to fetch | — | Not useful alone; used only for *timing* (§3.4) |

Even with a perfect prediction, the biggest visible win is §2 (cache before delay plus fewer round trips). Prefetch
then turns a cache *hit* into the normal case.

## 2. Phase 1: make the existing fetch fast and polite (do first, low risk)

1. **Cache before delay.** In `lyrics::fetch`, read `lyrics_cache::load` first. On a hit, post immediately and skip
   the 2 s wait. The delay stays only before network work. The gate still runs first in `lyric_filter::reconcile`,
   so a cache hit does not bypass Never/Off.
2. **Names lazily.** Search LRCLIB with the player's names first. Only call `localized::lookup` (Apple JP) when that
   finds nothing synced, then search again with the native names. That removes one request for the common case.
3. **Typed outcomes.** Replace `Result<Option<Found>>` inside the search with:
   ```rust
   enum Outcome { Found(Found), Instrumental, NotFound, Failed(Error) /* network, 5xx, 429 */ }
   ```
   - Retry rounds apply to `Failed` only, with backoff (1 s, 2 s, 4 s … plus jitter) and honouring `Retry-After`
     on 429 (LRCLIB docs say clients **must**).
   - `NotFound` stops after one round.
   - `Instrumental` shows "Instrumental" in the panel and pill and is cached.
4. **Negative cache.** In `lyrics_cache`, store misses as `{ kind: "miss" | "instrumental", at, duration_ms }`.
   - Miss TTL is 3 days. LRCLIB says a 404 may be filled by its background fetcher later.
   - Instrumental never expires.
   - "Search again" and manual search always bypass it.
   - Misses count toward `MAX_FILES` pruning. "Clear cache" removes them too.
5. **One shared client with timeouts.** Add `src/lyrics_net.rs` with:
   - a lazily-built `HttpClient`, reused by every provider;
   - `get(url, timeout)` that calls `.Cancel()` on the WinRT async op after 8 s;
   - a per-provider minimum gap (LRCLIB 250 ms, NetEase 500 ms) so prefetch bursts stay polite.

   Moving `get`/`check_request` out of `lyrics.rs` also takes that file back under the 800-line ceiling as
   features grow.
6. **Identify the client.** Change the UA to `SuiNotch/<CARGO_PKG_VERSION> (+https://github.com/giran03/SuiNotch)`
   and optionally register for an `Lrclib-App-Id` header (public, client-side). That doubles or quadruples the rate
   limit, which prefetch needs.
7. **Run LRCLIB `get` and `search` together** for the first name instead of one after the other. Use two threads
   joined with a deadline; `get` wins when both are synced.

Tests (pure, no network): outcome → retry decision table; backoff schedule; negative-cache TTL/expiry/bypass; Entry
JSON with the new `kind` stays compatible with old files (`#[serde(default)]`); cache-hit path posts without delay
(inject a clock/sleeper).

## 3. Phase 2: next-track prefetch

### 3.1 Learned transitions: `src/track_history.rs` (new, ~200 lines)

```rust
struct Next { key: u64, title: String, artist: String, album: String, duration_ms: u64, count: u32, last: i64 }
struct History { after: HashMap<u64 /*track_key*/, Vec<Next>> }   // saved as transitions.json
```
- Record in `reminders::on_event(Event::Media)`, where the track change is already detected (next to
  `app.recent.played`):
  - If the previous track was more than 80 % played, it is a natural transition with weight 2.
  - If it was skipped, weight 1.
  - Pause gaps longer than 30 min do not count.
- Keep the 4 most frequent successors per track and 3 000 tracks LRU. Decay counts by half every 90 days so old
  playlists fade.
- `predict(key) -> Option<&Next>`: the top successor if its score is at least 2 and clearly ahead of the runner-up
  (at least 1.5×). Shuffle produces flat distributions, so it naturally predicts nothing.
- Privacy: local only, like `recent.json`. Add it to PRIVACY.md and to "Clear history". Add a setting
  "Learn what plays next" (default on).

### 3.2 Prefetch worker: `src/lyrics_prefetch.rs` (new)
- A separate **lane** from the foreground. Foreground cancels through `GENERATION`; prefetch has its own
  `PREFETCH_GENERATION`.
  - Prefetch never posts `Event::Lyrics`. It only writes `lyrics_cache` (hits, misses, instrumentals).
  - A foreground fetch starting while a prefetch is running pauses the prefetch between requests. The foreground
    never waits on prefetch.
- Generalise `REQUEST_GENERATION` to `enum Lane { Foreground(u64), Prefetch(u64) }` so `check_request` aborts the
  right one.
- At most 1 prefetch running and at most 1 queued; it is low priority. Budget: at most 30 prefetches/hour, and
  none on a metered connection (`NetworkInformation::GetInternetConnectionProfile().GetConnectionCost()`). Pause
  while offline (`Event::Network`).
- Fetch the same things the foreground would (`Options::from_config`: fallback, extras, word timing). The cached
  entry is then complete and the extras call is not repeated on play.
- Skip if already cached (hit or unexpired miss).

### 3.3 Album-next, lazily
When there is no learned prediction and the current track has `TrackNumber` and `AlbumTrackCount` (new SMTC fields,
§4.1):
- look up the album once in Apple's store (`entity=song&attribute=albumTerm`, cached in memory per album);
- prefetch only track `n+1`.

That is one store request per album and one LRCLIB request per track, spread over the album, not a burst.

### 3.4 When to prefetch
Prefetch only when all of these hold:
- the current song's own lyrics have settled (found, miss or instrumental);
- it has played at least 30 s, or 40 % of its length, whichever comes first, so rapid skipping costs nothing;
- the predicted track is **gate-eligible** (§4) for the same player. A prediction made while a browser plays is
  never prefetched, because browsers are always Manual.

### 3.5 Duration of a song not yet playing
Predictions carry the `duration_ms` seen last time. Learned ones always have it. For album-next, Apple's
`trackTimeMillis` is used. `lyrics_cache::fits` keeps protecting against a different edit.

Tests:
- transition recording: natural vs. skip weights, pause gap, LRU cap, decay;
- `predict` thresholds, including a shuffle-like flat distribution giving None;
- lane cancellation: foreground cancel does not kill prefetch, and the reverse;
- budget and metered rules;
- prefetch writes the cache and never emits events;
- a native ignored test like `notch::lyrics_test`: play A, then B, then A, then B, and check that B's lyrics show
  with no network request (fake provider).

## 4. Phase 3: a better "is this a song?" gate

Keep the per-player `Policy` as the outer gate (your 1.0.4 decision stands: browsers are Manual/Never only). Inside
**Automatic**, replace the boolean `automatic()` with a scored, explainable classifier.

### 4.1 Read more metadata (`media.rs`, `events.rs`)
Add these to `Track`/`MediaInfo`, read once per track change:
- `album_artist`
- `track_number`
- `album_track_count`
- `genres: Vec<String>`
- `subtitle`

They come from the same `TryGetMediaPropertiesAsync` call already made. `track_key()` stays title+artist so cache
and shifts keep working.

### 4.2 `src/song_gate.rs` (new, pure, heavily unit-tested)
```rust
pub enum Verdict { Song, Unsure, NotSong }
pub struct Assessment { pub verdict: Verdict, pub score: i32, pub reasons: Vec<Reason> }
pub fn assess(info: &MediaInfo) -> Assessment
```
Signals (weights tuned with a fixture table of real metadata from Spotify music/podcast/ad, Apple Music, YouTube
Music PWA, foobar, VLC, MPC-HC, Windows Media Player):

| Signal | Weight |
|---|---|
| `PlaybackType == Video` | hard NotSong (as today) |
| Genre contains podcast / audiobook / spoken word / talk / news / comedy | hard NotSong |
| Title equals an ad marker ("Advertisement", "Spotify", empty artist with a sponsored title) | hard NotSong |
| `track_number > 0` **and** an album | +3 |
| Album present / album artist present | +1 / +1 |
| Artist ends with " - Topic" or contains "VEVO" | +2 (YouTube music channels) |
| Known music-player app (today's list) | +2 |
| Title episode patterns: `Ep. 12`, `Episode`, `#123`, `Chapter`, `Part 3 of`, `S2E5`, `Podcast` | −4 |
| Video-title patterns: `|` separators, hashtags, `vs`, "reaction", "tutorial", "trailer", "highlights", "livestream", more than 90 characters | −2 each, cap −4 |
| Duration 0 (live stream) | −2 |
| Duration under 30 s | −3 (jingles, ads) |
| Duration over 20 min | −2. A weak signal, never decisive alone, which keeps long live sets possible |
| Empty artist | −3 |

The thresholds are `Song` at a score of at least 2, `NotSong` at −3 or below, and `Unsure` between them.

### 4.3 Catalog verification for `Unsure` (the "much better method")
Metadata heuristics guess. The reliable signal is whether a **music catalog** has a recording with this title,
artist and length (±3 s).
- On `Unsure`, call Apple's store search (already used by `localized.rs`, no key, sends nothing beyond what
  LRCLIB already gets).
- A duration-matched hit makes it `Song`, then the normal fetch runs. It also hands the native-script names
  straight to §2 step 2, so it costs nothing extra.
- No hit means `NotSong` for this track, with the reason "Not found in a music catalog".
- Cache verdicts per track key (in memory, plus a small LRU on disk) so it runs once per track.

The cheapest high-precision check is the lyrics lookup itself. LRCLIB `/api/get` with `duration` (±2 s) rarely
false-matches. A variant could skip verification and treat a duration-matched LRCLIB hit as confirmation. That
contacts the lyrics service for unconfirmed media, though, which the 1.0.4 filter set out to avoid, so the
catalog step is preferred.

### 4.4 Surface the reason
`lyric_filter::State` keeps the latest `Assessment`. The lyrics panel already shows "Automatic lyrics skipped for
this source". Make it specific, for example "Skipped: looks like a podcast episode", with **Search lyrics anyway**
unchanged. Debug page (§5.4) shows score and reasons.

### 4.5 Policy wiring
- **Automatic + known music player:** `assess` can still veto, for Spotify podcasts and ads. Mostly a no-op for
  music.
- **Automatic + other non-browser player** (opted in): `Song` fetches; `Unsure` verifies through the catalog
  (§4.3); `NotSong` skips.
- **Browser:** unchanged, Manual/Never. Optional, **your decision:** a hint state where a high-confidence `Song`
  makes the button read "Looks like a song: Search lyrics" without searching automatically. That keeps the
  explicit-consent rule.

Tests: a fixture table (`song_gate_test.rs`) of about 40 real-world metadata rows with the expected verdict; reason
text; catalog duration matching; verdict cache; policy wiring matrix extending `lyric_filter` tests.

## 5. Phase 4: extra features worth adding

1. **LRCLIB word timing** from `lyricsfile` when `hasWordSync` is true. Parse `lines[].words` into the existing
   `words::Word`, then vet with `words::vet`. That gives word-by-word lyrics without the NetEase encrypted
   endpoint, and the extras call is skipped when LRCLIB already has words. The format needs a spike: the docs point
   to the Lyricsfile spec and the LRCGET source.
2. **Local `.lrc` folder.** Settings → "Lyrics folder". Look for `Artist - Title.lrc`, then `Title.lrc`,
   normalised, before any network call. It is the best source for local libraries and user corrections, works
   offline, and sends nothing.
3. **"Wrong lyrics" / remember match.** In the panel menu:
   - *Wrong lyrics* blocklists that LRCLIB id or NetEase id for this track key (`lyrics.json`, next to `Shifts`)
     and searches again;
   - *Keep these* pins the match so the cache is never replaced.

   Today "Search again" cycles but forgets.
4. **Lyrics diagnostics** in About → Debug (`notch/debug.rs` already exists). For the last 10 lookups, show:
   - the gate verdict and reasons;
   - provider, rounds and the time per request;
   - cache hit/miss/prefetch;
   - rate-limit waits.

   Local only. This makes the gate tunable from real use.
5. **Offline queue.** On `Event::Network { connected: false }`, foreground lookups stop retrying. When the
   network returns, the current track is searched again once, if it is still eligible and not yet found.
6. **Optional: Spotify queue (exact next track).** Opt-in setting, PKCE with a loopback redirect, scopes
   `user-read-playback-state`. Poll `/v1/me/player/queue` only on track change, and feed its first item to the
   prefetcher with the top priority. It needs a Spotify client ID, and Spotify's development-mode user cap. Decide
   only after Phases 1–3 show whether learned prediction is enough.

## 6. Files

| File | Change |
|---|---|
| `src/lyrics.rs` | cache-first, lazy names, `Outcome`, instrumental, parallel get/search; moves HTTP out |
| `src/lyrics_net.rs` *(new)* | shared client, timeout, per-provider pacing, `Retry-After`, lanes |
| `src/lyrics_cache.rs` | `kind` (hit/miss/instrumental), TTL, pinned flag |
| `src/lyrics_prefetch.rs` *(new)* | queue, budget, metered/offline checks, cache-only writes |
| `src/track_history.rs` *(new)* | transitions, predict, decay, persistence |
| `src/song_gate.rs` *(new)* + `src/song_gate_test.rs` | classifier, catalog verification, verdict cache |
| `src/media.rs`, `src/events.rs` | new SMTC fields |
| `src/lyric_filter.rs` | use `song_gate`, store `Assessment`, trigger prefetch at the right moment |
| `src/notch/reminders.rs` | record transitions, prefetch trigger, instrumental state |
| `src/pages/mod.rs` | specific skip reason, "Instrumental", wrong/keep actions |
| `src/settings/pages.rs`, `src/config.rs` | "Prefetch next song's lyrics", "Learn what plays next", "Lyrics folder" |
| `src/notch/debug.rs` | diagnostics |
| `PRIVACY.md`, `docs/lyric-filtering.md`, new `docs/lyrics-prefetch.md`, `changelogs/v1.0.4.md` (or v1.0.5) | docs |

## 7. Order, size and risk

| Phase | Size | Risk | Ship alone? |
|---|---|---|---|
| 1. Fast and polite fetch | M | Low: behaviour preserved, fewer requests | Yes, the biggest felt win |
| 3. Song gate (before prefetch, because prefetch reuses `assess`) | M | Medium: false negatives skip real songs. Mitigated by "Search anyway" and diagnostics | Yes |
| 2. Prefetch (learned plus album-next) | M–L | Medium: extra requests, so budgeted, metered-aware, cache-only | Yes |
| §5.1–5.5 Extras | S each | Low | Individually |
| §5.6 Spotify queue | L | High (OAuth, ToS, user cap) | Decide later |

Suggested order: **Phase 1 → Phase 3 (gate) → Phase 2 (prefetch) → §5.1, 5.3, 5.4 → §5.2, 5.5 → (§5.6?)**. Each phase follows the repo's TDD habit: pure
functions first with unit tests, then one ignored native test using disposable `APPDATA` and a fake provider (no
real network).

## 8. Open decisions for you

1. Browser hint (§4.5): allow a "Looks like a song" hint for browsers, or keep the button wording neutral?
2. Prefetch default: on by default (budgeted, local learning), or opt-in?
3. Register SuiNotch with LRCLIB for an `Lrclib-App-Id` (higher limits, a contact channel)?
4. Spotify queue integration: worth an opt-in phase, or out of scope?
5. Target version: fold Phase 1 into 1.0.4 (unreleased), and Phases 2–4 into 1.0.5?
