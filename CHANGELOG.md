# Changelog

All notable changes to SuiNotch. Newest first.

## [1.0.8] - 2026-11-10

### Changed
- The tray icon's right-click menu has a new look that matches the Settings window: its dark surface colour, an icon beside every entry, divider lines between groups, accent-coloured checkmarks and a red Exit entry.
- The tray menu opens upward or downward depending on which side of the screen the taskbar is on.

## [1.0.7] - 2026-11-09

### Added
- Another backup lyrics service under Fall back to other services. Its lyrics time every word, so many more songs get a word highlight that follows the singer instead of an estimate, and songs missing elsewhere can still be found. It is sent the song's title, artist and length.
- A song title too long for the closed notch goes round while the music plays, its start following its end after a short gap.
- A lyric line too long for the notch scrolls through once, with its romaji or translation after it, in time with the singing, instead of being cut off or leaving the romaji out.

### Changed
- The Scroll fit for long lines scrolls to the end once, in time with the singing, instead of going back and forth.
- Romaji reads as words ("anata ga suki") instead of one syllable at a time ("a na ta ga su ki").
- Only English translations are shown. Translations into other languages are left out, and romaji still shows.
- The backup lyrics services are no longer named in the app; lyrics from them are labelled "Backup service".
- The word highlight follows the singer exactly on lyrics with word-by-word timing in the lyrics themselves (enhanced LRC), from LRCLIB or your own .lrc files, instead of estimating it from the text.

### Fixed
- A song with an English title is no longer searched for under another song's Japanese name. A Japanese title from Apple's store is only used when Apple's English name for that same track is the song's title, by the same artist ("JET" was being searched as ジェット・ラグ, "Jet Lag").
- Lyrics are looked up again when the player gives a song's length a moment after its name. The first search went out with the previous song's length, which could match a different song.
- Romaji and translations from another song with a similar name ("JET" showing "Jet Lag"'s romaji) are no longer shown: they are only taken when the backup service's lyrics are the same as the ones shown, and each sits under the line with the same words instead of the line at about the same time. Songs already saved are looked up again.
- Lyrics with word-by-word timing (enhanced LRC) no longer show raw timestamps such as `<01:54.955>` in the middle of a line.
- The Settings window's close and minimize buttons work again on the Music and lyrics page.
- The word highlight no longer rushes to the end of some Japanese, Chinese and Korean lines. Word timing now fits far more of their lines (kanji written two ways, lines broken differently between services), and lines without it are paced by their syllables instead of their letters. Songs already saved are looked up again for the better timing.

## [1.0.6] - 2026-11-08

### Added
- Right-click the volume button or volume slider on the Music tab to pick which audio source will be adjusted, including active output playback devices and specific apps (per-app volume mixer).
- Per-app audio mixer support: choose running applications (such as Spotify, Discord, Chrome or System Sounds) or output endpoints from the context menu. Adjusting the slider, mouse wheel scrolling or mute button directly controls that app's volume session.
- Context menus for volume targets display matching icons for headphones, displays, speakers, apps and system sounds, with an accent checkmark on the selected target.
- Auto option for lyrics mode that reapplies your chosen lyrics style preset from Settings.

### Changed
- The volume icon and slider knob and fill tint with the notch accent colour when adjusting an app or non-default audio source, clearly indicating per-app mixer mode.
- Volume tooltips show the active target app or source name along with the right-click shortcut hint.

## [1.0.5] - 2026-11-04

### Added
- Right-click the lyrics mode button to choose Outside, Inside, Both or Off, or the audio source button to choose an open player directly. Menus mark the current choice; left-click still cycles.

### Changed
- The Music volume slider opens horizontally when lyrics are closed and there is enough room beside the playback controls. It stays vertical with lyrics open or in a compact layout.
- Public Google Calendar embed links are converted to their iCal feed, including links saved by older versions. Calendar web pages and failed requests now show an error instead of a successful empty sync.
- Music context menus also match the notch's outer corner roundness and remove the native border outline.
- Lyrics and audio-source menus follow the notch's dark appearance, accent colour and corner settings. Each lyrics mode has its placement icon, with an accent checkmark on the current choice.
- Temporary notifications appear below the notch in the outside lyrics area, briefly replacing outside lyrics until the notification disappears.
- Non-browser players now default to Automatic lyric searches. The per-player setting is labelled Lyric search mode without including the player's name. Saved Manual and Never preferences are respected; browsers still require an explicit search.
- Lyrics searches retry unsuccessful lookups up to the configured retry count, stopping when lyrics or an instrumental result is found. Previous cached misses and missing music-catalog matches no longer block the current song's search.

## [1.0.4] - 2026-11-03

### Added
- About now includes Debug actions for an Updates tab simulation, sample volume HUD and notification. Update previews close when you leave or collapse the tab, and never download or install anything.
- Lyrics search only for likely songs by default, with a short playback delay, saved per-player Automatic / Manual / Never preferences, and Search lyrics anyway for uncertain sources. Browsers require manual search; Lyrics Off stops fetching while retaining already-loaded lyrics.
- The lyrics icon cycles through Outside, Inside, Both and Off. Both shows the current lyric in the notch and below it at the same time; the chosen mode is saved and reflected in Settings and the live preview.
- The next song's lyrics are fetched ahead while the current one plays, so they show the moment it starts. SuiNotch learns on this PC what tends to play next, or uses the next track of the album; it is limited to 30 lookups an hour and skipped on metered connections. Turn it off with Fetch the next song's lyrics ahead, or stop the learning with Learn what plays next.
- A song check skips podcasts, audiobooks, ads, jingles and videos in automatic lyric searches, including podcasts in Spotify. Players you set to Automatic are checked against a music catalog when their metadata leaves doubt. The lyrics panel says why a track was skipped.
- Your own lyric files: put .lrc or .txt files named "Artist - Title" in the new lyrics folder (Settings > Music and lyrics > Your own lyric files) and they are used first, offline.
- Wrong lyrics and Keep buttons in the lyrics tool bar. Wrong lyrics are never offered for that song again; kept lyrics stay whatever later searches find.
- Instrumental tracks say "Instrumental" instead of "No lyrics found".
- Word-by-word timing from LRCLIB for songs that have it, without asking a backup service.
- Diagnostics lists the last lyric lookups and the song check for the track playing.
- A temporary Updates tab appears when an update is available. Download and verify the update, then install and restart when ready. The tab shows downloading, ready, installing and retry states and hides after completion.

### Changed
- Lyrics already known show at once instead of after the two-second wait, and fewer requests are made. The native-script name is only looked up when needed. LRCLIB's exact match and search are asked together. A song with no lyrics is remembered for three days instead of being searched six times on every play. Retries now happen only when a service fails to answer, waiting longer each time and honouring the service's request to slow down. Every request times out after 8 seconds.
- When the network comes back, a song whose lyrics could not be fetched is searched again.

### Fixed
- The privacy notes now list the backup lyrics services and Apple's store, which the lyrics fallback (on by default) contacts.
- Cycling lyric placement or Off/On preserves already-found lyrics and their translations/word timing for the current song. Browser sources require an explicit lyric search, including those with an older Automatic preference; toggling display options never starts a browser search.
- The notch restores its always-on-top position after switching windows, reappearing or resuming, without taking keyboard focus. A lightweight recovery check also handles apps that restack later, while preserving full-screen hiding.
- Sound previews change only the clicked Play button to Stop, even when several reminders use the same sound.
- Closing the completed Updates tab returns to the page you were using without opening a hidden page.

## [1.0.3] - 2026-11-02

### Added
- Reminder sounds with imports, word-by-word lyrics, calendar tasks, browser image drops and settings reset (bde0772)
- Lyrics fallback, look options, second line, tooltips and settings tools (6e3335f)

## [1.0.2] - 2026-10-30

### Added
- Notch styles (docked, floating, flat) with a floating gap setting (0b890bc)

## [1.0.1] - 2026-10-30

### Added
- Clipboard page, history across restarts and per-page toggles (8d5abd6)
- Timers, stats, clipboard history, calendar events and more (00ab40e)
- Keep the focus timer when previewing another duration (432e2e5)
- Switch between open media players from the Music page (0b40781)

### Changed
- Split notch and settings modules, log panics, add CI (dc1956c)

### Other
- Shorten the About tagline (aba9821)
