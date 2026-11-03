# Lyrics button modes

The lyrics icon on the Music page cycles through:

1. **Outside:** show the current lyric in the floating display below the closed notch.
2. **Inside:** show the current lyric inside the closed notch, replacing the song title.
   The expanded Music page opens its full lyrics panel.
3. **Both:** show the current lyric inside the closed notch and below it simultaneously.
   The expanded Music page also opens its lyrics panel.
4. **Off:** hide both displays and the expanded lyrics panel.

Click again after Off to return to Outside. The button reflects the current saved
mode, so an existing Outside selection advances to Inside on the next click.
Its icon shows the placement and its tooltip explains the next click.

The choice uses the existing visibility and placement settings and survives a
restart. Settings > Music and lyrics > Placement includes Both, and the live preview
shows both displays together. Existing colour, highlight, size, animation and
second-line choices are preserved when cycling modes. Loaded lyrics, translation, romaji and word timing stay in memory through all four modes, including Off, so returning to Outside restores the same result without another lookup. Off cancels pending work.

Browser sources require an explicit **Search lyrics anyway** for the current track; the placement toggle never authorizes a browser search. The loaded manual result survives Off/On, and is cleared when the track or source changes.

Floating lyrics continue to follow the existing playback, pause, alert and notch
expansion rules. They appear when synced lyrics are available and the notch is
closed. Off stays off even if the automatic lyrics-panel preference is enabled.

The isolated native test renders all four modes, open and closed, at 100%, 150% and
200% scaling without playing audio or making network requests:

```powershell
$env:APPDATA = Join-Path (Get-Location) 'target/lyrics-toggle-test'
cargo test --locked lyrics_button_cycles -- --ignored --test-threads=1
```

Run this in a separate shell so the disposable APPDATA setting does not carry into
normal app launches. Snapshots are saved under that directory.
