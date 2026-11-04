# Development target

The current development version is **1.0.6**. Version 1.0.5 is frozen at tag
`v1.0.5`; record all upcoming changes in `changelogs/v1.0.6.md`. The app embeds
the notes matching the version in `Cargo.toml` through `build.rs`.

Add release-note entries as changes are implemented. Keep proposed work here
until it is implemented, so the app's What's new page describes shipped behavior.
Update the main `CHANGELOG.md` when preparing the next release.

## Candidates for 1.0.6 planning

These deferred ideas are routed to 1.0.6 planning; they are not promised release scope:

- Optional Spotify queue integration for exact next-track lyric prefetch.
- Optional LRCLIB app registration.
- Scheduled clock alarms, recurrence and sleep handling.
- Quiet-mode exceptions for sounds.
- Sound themes, per-event volume and per-task sound overrides.

The [lyrics fetching plan](plans/lyrics-fetching-plan.md) is implemented in 1.0.4
except its optional Spotify queue integration. Its opening status records the
decisions that supersede the original open questions near the end of that plan.
