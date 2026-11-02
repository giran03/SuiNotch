# Reminder sounds

Open **Settings > Sounds** (also linked from General) to choose sounds for focus,
breaks, timers, tasks, calendar events, water and low battery. Existing installations
keep their Bell, Soft and Sui Drop assignments and their sound/quiet preferences.

Eight original synthesized presets are included: Sui Drop, Bell, Soft, Glass, Ripple,
Spark, Beacon and Classic Ring. Each has a duration and Play/Stop preview. Choose
**Silent** to mute one reminder type, or use the master switch and volume to control
all reminder audio. Previews respect the master mute and volume; clicking Play is
an explicit preview and does not wait for Do Not Disturb to end.

## Importing audio

Use **Import sound > Browse**, or drop one WAV, MP3 or OGG Vorbis file onto the Sounds
page. Audio is decoded and checked in the background. Sources can be up to 20 MB and
five minutes long, with one or two channels at 8–96 kHz. Imported clips initially
use the first 30 seconds (or the complete source if shorter).

Use **Edit** beside an import to rename it, change the clip's start and length in
one-second steps, enable optional peak loudness adjustment, replace its source, or
delete it. Clips are at most 30 seconds. Edge fades reduce clicks at trim boundaries.
Rename accepts Enter to save and Escape to cancel. Replacement preserves the name
and reminder assignments. Deletion requires a second click and lists affected
reminders, which return to their original default sounds.

Imports are copied into `%APPDATA%\SuiNotch\sounds`; moving or deleting the original
source does not affect them. Settings store names, clip selections and assignments.
No audio is uploaded. Missing or unreadable imports fall back to the event's default
chime and keep the visual reminder. Restore sound defaults resets assignments while
keeping imports. Reset all settings also preserves imported audio as user data.

## Repeating timers

Enable **Repeat timer sound** for countdown timers. Set a 1–30 second pause between
rings, a 5–300 second maximum ringing duration, and optional gradual volume increase.
Repetition is off by default. Ordinary reminders play once.

Only one sound plays at a time. Timer alerts take priority; previews cannot interrupt
an alert. Dismissal, snooze, restart, mute and sound configuration changes stop the
current alert audio. Closing Settings or leaving Sounds stops previews.

Do Not Disturb is respected by default, including during a repeating timer. Changing
the existing quiet preference applies to sounds as well. This release does not add
scheduled clock alarms, recurrence, wake-from-sleep or missed-alarm handling.

## Verification

Normal checks: `cargo build --locked`, `cargo test --locked`, and
`cargo clippy --locked --all-targets`.

The opt-in native sound tests require a disposable `APPDATA` directory ending in
`sound-test` and a working Windows audio output device. They use original sine-wave
fixtures, render settings at 100%, 150% and 200%, and play only silence during the
device test. They never use desktop input or the user's real app data:

```powershell
$env:APPDATA = Join-Path (Get-Location) 'target/sound-test'
cargo test --locked sound -- --include-ignored --test-threads=1
```

Run that command in a separate shell so its isolated APPDATA setting does not affect
subsequent normal launches. The output-device test may require execution outside a
sandbox that restricts Windows audio handles. Fixture audio in `src/sound/fixtures`
is an original 440 Hz sine tone encoded as WAV, MP3 and OGG for decoder regression tests.
