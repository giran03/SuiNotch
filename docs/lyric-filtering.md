# Lyric search filtering

Automatic lyric searches wait two seconds after eligible playback starts before using the network; a song already known (your own lyric file, the cache, or a remembered "nothing found") shows at once. See [lyrics fetching](lyrics-prefetch.md) for the order of lookup, fetching ahead, and wrong or kept lyrics. The default permits recognized music players (Spotify, Apple Music, foobar2000, MusicBee, AIMP, TIDAL, Deezer, Amazon Music and Qobuz). Browsers and unknown or general-purpose media players use manual search by default, even if Windows reports Music.

## Song check

Inside Automatic, each track is weighed by what the player says about it (`src/song_gate.rs`):

- **Settled outright:** video playback, a podcast, audiobook or spoken-word genre, an ad title, or a missing title or artist skip the track.
- **Points for a song:** a track number on an album, an album, an album artist, a music channel ("- Topic", VEVO), a music player.
- **Points against:** episode-like titles ("Episode 12", "Ep. 4", "#123", "S2E5", "Part 2 of 5", "podcast"), video-like titles (" | ", hashtags, "vs", "reaction", "trailer", "highlights", very long titles), no length (a stream), under 30 seconds, over 20 minutes.

Two points or more is a song, minus three or less is not, and anything between is unsure. A recognized music player is trusted when unsure, but an episode title still outweighs it, so Spotify podcasts are skipped. A non-browser player you set to Automatic that leaves doubt is checked against Apple's music store first: a recording with that title (or artist) and a length within three seconds confirms it, and its native-script name is used for the search. No match skips the track with "Not found in a music catalog", and the answer is remembered for the session. With Fall back to other services off, the catalog is never asked and unsure tracks are searched as before. The lyrics panel shows why a track was skipped; Settings > About > Diagnostics shows the score and every sign.

In Settings > Music and lyrics > Lyric searches, click the current player's preference to cycle Automatic / Manual / Never. Preferences persist per Windows player ID. Explicitly choosing Automatic opts non-browser players into metadata-based detection. Detected browsers are always Manual or Never; even an older saved Automatic preference cannot bypass the browser gate. Manual permits only a deliberate Search lyrics anyway; Never blocks manual searches too. Lyrics Off prevents all lyric searches.

Skipped sources show an explanation and Search lyrics anyway in the lyrics panel. Manual search does not change the saved source preference. It applies only to the current source and track, and survives display toggles for that track. Browser display-setting changes never launch an extras lookup. The first manual lookup uses the best match; Search again advances to alternatives.

Changing tracks or sources, stopping playback, or choosing Never invalidates pending results and clears lyrics. Turning lyrics Off cancels pending work but keeps the current track's completed lyrics, translations, romaji and word timing in memory; toggling back on restores them without another lookup. Placement changes also preserve the loaded result. Pausing cancels pending automatic work; resuming can start again. Completed lyrics remain while paused. Config-triggered translation and word-timing lookups respect the same permissions. Already-issued HTTP requests may finish, but cancelled results are discarded and further retry rounds stop.

Windows metadata cannot reliably distinguish songs, podcasts, advertisements and videos in every player. Music is a hint, not proof: browsers may report it for any media. Length only adds or takes points; it never decides alone, so short songs and long performances still pass in a music player. Browser blocking applies to the entire player, not a website; distinguishing YouTube Music from social sites needs a future browser integration with domain information. Use Manual or Never for sources whose metadata is unreliable.

## Validation

Run `cargo test --locked --offline` for the regular suite. Native checks use disposable data and require Windows graphics:

```powershell
$env:APPDATA = Join-Path (Get-Location) 'target/lyrics-toggle-test'
cargo test --locked --offline notch::lyrics_test -- --ignored --test-threads=1
$env:APPDATA = Join-Path (Get-Location) 'target/sound-test'
cargo test --locked --offline lyric_source_preferences_render_and_persist -- --ignored --test-threads=1
```

The native checks cover cancellation, source switches with identical song names, rejected stale results, manual permissions, Never, pause/resume, Lyrics Off, preference persistence and UI rendering at 100/150/200%. They do not contact lyric services or control real media playback.
