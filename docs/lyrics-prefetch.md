# Lyrics: fetching, fetching ahead, and your own files

## Order of lookup

For the song playing, SuiNotch tries, in order:

1. **Your own lyric files.** These are `.lrc` (synced) or `.txt` (plain) files in `%APPDATA%\SuiNotch\lyrics`, named `Artist - Title` or `Title`. Case, spaces and punctuation are ignored, so `acdc - tnt.lrc` matches "T.N.T." by AC/DC. Open the folder from Settings > Music and lyrics > Your own lyric files. These files never send anything.
2. **The cache**, with Keep lyrics on this PC turned on:
   - Synced lyrics show at once.
   - A song with nothing found is remembered for three days, because LRCLIB adds missing songs in the background.
   - An instrumental is remembered for good.
   - A previous miss does not block a fresh search for the current song. Lookups ahead still skip cached misses to conserve their budget.
3. **LRCLIB**, under the player's names. The exact match (`/api/get`, by name and length) and the search are asked side by side.
   - Synced lyrics win.
   - An exact match marked instrumental shows "Instrumental".
   - Records with word timing (`hasWordSync`) give word-by-word lyrics without a backup service.
4. **Native-script names** from Apple's store, only when the first try found nothing synced.
5. **Backup lyrics services**, with Fall back to other services.

A song's first search repeats unsuccessful rounds up to Keep searching for (5 retries by default, or 6 total rounds), including when services answer "no such song". It stops as soon as lyrics or an instrumental result is found. Zero retries means one round. Search again advances to an alternative match in a single round. The wait doubles each retry (1, 2, 4, 8 s) plus jitter. A service's `Retry-After` is honoured, and every request has an 8-second timeout. Requests to one service are spaced at least 250 ms (LRCLIB) or 150 ms (others) apart. Changing track, pausing, turning lyrics Off or changing source permissions cancels pending automatic work. The User-Agent names the app, its version and its homepage, as LRCLIB asks.

When the network comes back, a track whose automatic lookup found nothing is tried once more.

## Fetching ahead

Windows media controls report only the current track, not the queue. The song likely to play next is worked out in two ways:

- **Learned (Learn what plays next):**
  - Each time one song follows another, that is noted in `transitions.json`. A song played through counts twice and a skip once.
  - Scores halve every 90 days. At most 4 successors are kept per song and 3 000 songs in all.
  - A song is predicted only with a score of 2 or more and a clear lead over the runner-up (1.5×), so shuffle predicts nothing.
  - A pause of more than 30 minutes between songs is not counted.
- **The album:** when nothing was learned and the player gives a track number and album, the next track is looked up in Apple's store (once per album), if Fall back to other services is on.

The lookup ahead:
- starts once the current song's lyrics have settled and it has played 30 s or 40 % of its length, whichever is sooner;
- only covers songs that would be fetched automatically anyway (never for browsers, never with a catalog check);
- runs at most one at a time, gives way whenever the song playing needs the network, and is cancelled when the track changes;
- is limited to 30 an hour and skipped on metered, capped or roaming connections and while offline;
- only writes the cache, never the screen, and needs Keep lyrics on this PC.

Turn it off with Settings > Music and lyrics > Fetch the next song's lyrics ahead. Clear recently played also clears what was learned.

## Wrong lyrics and keeping lyrics

The lyrics panel's tool bar (shown on hover) has two buttons:

- **Wrong lyrics (thumb down):** the lyrics are never offered for that song again, and the best other match is looked up. The marks are kept in `lyrics.json`, at most 20 per song.
- **Keep (pin):** synced lyrics stay for the song whatever later lookups find, and are never pruned from the cache. Click again to stop keeping them.

Neither appears for your own files.

## Diagnostics

Settings > About > Diagnostics shows:
- the song check for the track playing;
- how many songs were learned;
- the last 10 lookups, now playing and ahead: the result, requests, time and any wait.

## Validation

```powershell
cargo test --locked --offline
$env:APPDATA = Join-Path (Get-Location) 'target/lyrics-toggle-test'
cargo test --locked --offline known_songs_resolve -- --ignored --test-threads=1
cargo test --locked --offline lyrics_button_cycles -- --ignored --test-threads=1
```

The ignored check stores songs and a miss in a disposable cache. It confirms cached lyrics resolve at once without a request, while a cached miss proceeds to the network wait and respects cancellation. No test contacts a lyrics service.

Run native graphics checks in separate test processes, as above. Combining several
window/graphics fixtures into one test process can stall after the first fixture.
